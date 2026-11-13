# V1 baseline: pools, routes, efforts and limits

Program: code#9399 (design mill). Status: audit of V1 as it exists on `origin/main` today.

**Sources**

- Config: `.opencode/opencode.jsonc` (3,731 lines). It is identical on `origin/main` (08c1834b78) and on this worktree (HEAD 5700c42349). All `file:line` cites below are to this file unless named otherwise.
- Class table and guard: `.opencode/route-fallbacks.md` and `bin/check-opencode-route-rules.mjs:30-38`.
- Fork expander and resolver: `~/git/opencode` `origin/HEAD` 25b50a485b, read via `git show`. Files: `packages/core/src/config/plugin/model-routes.ts` (cited "MR"), `packages/core/src/model-resolver.ts` ("MRes"), `packages/core/src/model-route.ts` ("MRoute"), `packages/schema/src/config/model-routes.ts` ("Schema").
- Live data: `route_usage`, 72 h (from 2026-11-10T12:04Z). Counts are per attempt on each provider/model leaf, not per session.
- Machine-readable companion: `baseline/v1-pools.json` (same directory).

**Caveats**

- Several providers are catalog-only in config: `opendesign` (names only), `cline-pass` (ids only), `_commandcode`, the `opencode-go*` instances, `_openai.*` (`instanceOf openai`), `_antigravity` and `mistral` (sync-models runtime overlay, per the comment at 269-272 and 501/511). Their variant sets and native limits are **not visible** here. Where this audit states a variant for them, it comes from a config comment, not from a declaration.
- The fork schema and expander are read at `origin/HEAD`, which may run ahead of the build the fleet is running. Behaviour claims are therefore "per fork source", not "per live binary".

---

## a. Every route and pool

Column key: **strategy** = `selection` (`ordered`, `round-robin`, `weighted`). `autonomy: "rules"` is set on every route (for example line 2309), so weights and window gates are honoured. Weights under round-robin are priority tiers, not shares (MR; line 2310 comment). `windows` = named windows used as gates (`model_route_windows`, lines 3690-3729).

### a.1 Class routes (the spine)

| Route | Line | Strategy | Targets, in order | Class fallback (trailing targets) | Windows |
|---|---|---|---|---|---|
| `small` | 3672 | ordered | `limited`, `speed` | `speed` | none |
| `speed` | 3544 | ordered | `_muse/muse-spark-1.3-contributor#xhigh` (3586), `deepseek-flash-plan#high` (3587), `gemini-hydrogen-plan#high` (3588), `haiku-plan#high` (3589) | `workhorse`, `summit`, `specialist` (3590-3592) | none |
| `workhorse` | 3176 | ordered | `workhorse-ordered`, `workhorse-primary`, `workhorse-weighted`, `workhorse-last-resort` (3225-3228) | `scout`, `specialist` (3229-3230) | none |
| `summit` | 3490 | ordered | `fable-plan#medium` (3533), `opus-plan#high` (3534), `argon-plan#high` (3535), `_antigravity/gemini-4-hydrogen#high` (3536), `_muse/muse-wonder-1.0-contributor#xhigh` (3537), `sol-plan#xhigh` (3538) | `specialist` (3539) | none |
| `specialist` (sink) | 3445 | ordered | `fable-plan#low` (3480), `sonnet-plan#high` (3481), `argon-plan#high` (3482), `spark-plan#xhigh` (3484), `sol-plan#medium` (3485) | none (sink) | none |
| `scout` | 2859 | ordered | `beam-plan` (2890), `deepseek-flash-plan#low` (2891), `haiku-plan#low` (2892) | `workhorse` (2893) | none |
| `background` | 2898 | ordered | `mimo-flash-plan` gated `deepseek-peak-early` (2949), `mimo-flash-plan` gated `deepseek-peak-late` (2953), `deepseek-flash-plan#high` gated NOT early (2957), `deepseek-flash-plan#high` gated early (2961), `mimo-flash-plan` (2964) | `workhorse` (2965) | `deepseek-peak-early`, `deepseek-peak-late` |

Class fallback matches `CLASS_FALLBACKS` in `bin/check-opencode-route-rules.mjs:30-38` (`small→speed`, `speed→workhorse,summit,specialist`, `workhorse→scout,specialist`, `summit→specialist`, `scout→workhorse`, `background→workhorse`). The brief's "Drew ruled" status is split: `small`, `speed`, `workhorse`, `summit` and `specialist` are confirmed in `route-fallbacks.md`; `scout` and `background` → `workhorse` are marked PROPOSAL there and in the comments at 2869-2872 and 2908-2911.

`background` and `limited` are the only two routes that use window gates on targets.

### a.2 Workhorse stages and trial routes

| Route | Line | Strategy | Targets (variant, weight) | Notes |
|---|---|---|---|---|
| `workhorse-ordered` | 3007 | ordered | `deepseek-effort-rr` (3021) | 60 s first-token timeout (3028) |
| `workhorse-primary` | 3030 | round-robin | **empty** (3167) | Emptied 2026-11-07; contributes nothing |
| `workhorse-weighted` | 3240 | weighted | `luna-effort-rr` w10 (3381), `_muse/muse-spark-1.3-contributor#high` w10 (3382), `_antigravity/gemini-4-hydrogen#low` w10 (3383), `_antigravity/claude-sonnet-5-5#medium` w10 (3384) | 60 s timeout (3398). Weight map 3387-3392 |
| `workhorse-last-resort` | 3400 | ordered | `_claude/claude-haiku-5-5#high` (3435), `fable-plan#low` (3436) | 60 s timeout (3443). Head swapped Sonnet→Haiku 2026-11-12 |
| `deepseek-effort-rr` | 2970 | weighted | `deepseek-flash-plan#high` w3 (3001), `deepseek-flash-plan#low` w1 (3002) | Effort trial. Review after 5,000 low draws or 7 days; revert if low success < 90% of high (2992-2997) |
| `luna-effort-rr` | 2481 | round-robin | `openai-plan/gpt-6-luna#medium` (2500), `openai-plan/gpt-6-luna#high` (2501) | Effort trial, 2026-11-12. Feeds workhorse-weighted |
| `limited` | 3597 | ordered | see a.4 | Light-work tier with per-target context caps |
| `deepseek-flash` | 2325 | ordered | `deepseek-flash-plan` (2339) | Pass-through wrapper; 60 s timeout (2346) |

### a.3 `*-plan` pools (leaves; variant remaps)

A **remap** is `targets[].variants` (`Schema:50-58`). It maps a caller's route-level variant to this leaf's variant, and is applied only when the door's model declares that variant (MR:1017-1021). Weights are the round-robin tiers in the route's own `weights` map unless marked "weight" on the target.

| Pool | Line | Strategy | Leaves (variant / remap / weight) | Note |
|---|---|---|---|---|
| `deepseek-flash-plan` | 2165 | round-robin | `opencode-go-plans/deepseek-v4.1-flash` w2, remap medium→high, xhigh→max (2284-2286); `cline-pass/deepseek-v4.1-flash` w1, remap medium→high, xhigh→max (2288-2290); `opendesign/deepseek-v4.1-flash` w1, remap xhigh→high, max→high (2292-2294); `mistral-plan/deepseek-v4-1-flash-1017` w1, remap low→low, medium→high, high→high, xhigh→max, max→max (2296-2306) | `_commandcode` removed 2026-11-13 (2167). Health 60 s (2323) |
| `deepseek-pro-plan` | 2348 | round-robin | `opencode-go-plans/deepseek-v4-pro` w3, remap medium→high, xhigh→max (2363-2365); `cline-pass/deepseek-v4-pro` w1, remap low→high, medium→high, xhigh→max (2366-2369); `opendesign/deepseek-v4-pro` w1, remap xhigh→high, max→high (2370-2373) | **ClinePass remaps `low` to `high`** (2368), silently |
| `fable-plan` | 2381 | ordered | `_claude/claude-fable-5-2` (2392) | Single door |
| `haiku-plan` | 2397 | ordered | `_claude/claude-haiku-5-5` (2416) | Single live door (Go and OpenDesign legs removed 2026-11-11, 2403-2409) |
| `sonnet-plan` | 2550 | round-robin | `_claude/claude-sonnet-5-5` (2569); `_antigravity/claude-sonnet-5-5`, remap xhigh→high, max→high (2570-2573) | |
| `opus-plan` | 2578 | ordered | `_claude/claude-opus-5-5` (2593); `_antigravity/claude-opus-5-5`, remap xhigh→high, max→high (2594-2597) | |
| `gemini-hydrogen-plan` | 2512 | round-robin | `_antigravity/gemini-4-hydrogen`, remap max→xhigh (2542-2545) | Single door |
| `luna-plan` | 2430 | round-robin, weights `opencode-go-plans/gpt-6-luna` 6, `openai-plan/gpt-6-luna` 2.5 (2476-2479) | `openai-plan/gpt-6-luna` (2467); `opendesign/gpt-6-luna`, remap xhigh→high, max→high (2468-2471); `opencode-go-plans/gpt-6-luna` (2472) | Consumed by `limited` as `luna-plan#medium` (3653) |
| `terra-plan` | 2692 | round-robin | `openai-plan/gpt-6.1-terra` (2728) | Consumed by `limited` as `terra-plan#low` (3654). The "orphan" comment at 2716-2727 is **stale** |
| `sol-plan` | 2647 | round-robin, weight `openai-plan/gpt-6.1-sol` 3 (2688-2690) | `openai-plan/gpt-6.1-sol` (2680); `opendesign/gpt-6.1-sol`, remap xhigh→high, max→high (2681-2684) | `_commandcode` removed (2649) |
| `spark-plan` | 2602 | ordered | `opencode-go-plans/muse-spark-1.3-contributor` (2640); `cline-pass/muse-spark-1.3-contributor` (2641); `_muse/muse-spark-1.3-contributor` (2642) | `_commandcode` removed (2604) |
| `argon-plan` | 2732 | round-robin, weight `opencode-go-plans/gemini-4-argon` 3 (2765-2767) | `opencode-go-plans/gemini-4-argon` (2760); `_antigravity/gemini-4-argon` (2761) | |
| `beam-plan` | 2769 | round-robin, weight 3 (2788-2790) | `opencode-go-plans/beam-free` (2784) | `_commandcode` beam:free removed (2771). Go beam-free is quota-walled until the 2026-11-15 Go reset (2774-2775) |
| `mimo-flash-plan` | 2792 | round-robin, weight `opencode-go-plans/mimo-v2.6-flash` 3 (2815-2817) | `opencode-go-plans/mimo-v2.6-flash` (2809); `cline-pass/mimo-v2.6-flash` (2810); `opendesign/mimo-v2.6-flash` (2811) | `_commandcode` removed (2794) |
| `glm-5-3-plan` | 2819 | round-robin | `_vibe/zai-glm-5-3`, remap xhigh→max (2834-2836) | **Defined only**: no route references it (2831) |
| `glm-5-3-flash-plan` | 2841 | round-robin | `_vibe/zai-glm-5-3-flash`, remap xhigh→max (2852-2854) | **Defined only** (2849) |

### a.4 `limited` and `small` (context-capped light tier)

`limited` (3597-3671) is the only route in the config that sets a per-target context cap. Every member and its cap:

| # | Member | Variant | Window gate | Cap (`limit.context`) | Line |
|---|---|---|---|---|---|
| 1 | `luna-plan` | medium | none | 100,000 | 3653 |
| 2 | `terra-plan` | low | none | 388,000 | 3654 |
| 3 | `mimo-flash-plan` | — | `deepseek-peak-early` | 1,048,576 | 3655 |
| 4 | `mimo-flash-plan` | — | `deepseek-peak-late` | 1,048,576 | 3656 |
| 5 | `deepseek-flash-plan` | low | NOT `deepseek-peak-early` | 1,000,000 | 3658-3661 |
| 6 | `deepseek-flash-plan` | low | `deepseek-peak-early` | 1,000,000 | 3663-3666 |
| 7 | `haiku-plan` | medium | none | 388,000 | 3667 |

`small` (3672-3688) is `limited` then `speed`. It has no route-level cap any more (the 200k cap was deleted on 2026-11-13; comment 3674-3682).

### a.5 Provider routes (`experimental.provider_routes`, lines 2074-2153)

| Provider route | Line | Members | Selection |
|---|---|---|---|
| `openai-plan` | 2097 | `_openai.alt`, `_openai.main` (2113) | round-robin, no weights (2114) |
| `mistral-plan` | 2126 | `_vibe`, `mistral` (2127) | ordered (2128): Vibe plan leg first, Mistral API as reserve |
| `opencode-go-plans` | 2149 | `opencode-go.drewrypope`, `opencode-go.popesdrew`, `opencode-go.developing-today-0` (2150) | weighted 1:1:1 (2151-2152) |

### a.6 Route-level settings that are not used

- `limit.context` on a route: **not set anywhere** (grep). The only caps are the target caps in `limited`.
- `cache_format` on a route or target: **not set anywhere**.
- `budgets`: **not set anywhere**.
- `health.firstTokenTimeoutMs = 60000` is set on 7 routes: `deepseek-flash-plan` (2323), `deepseek-flash` (2346), `workhorse-ordered` (3028), `workhorse-primary` (3174), `workhorse` (3238), `workhorse-weighted` (3398), `workhorse-last-resort` (3443). It was added for task 14574 (#7927).

### a.7 Drift between `route-fallbacks.md` and the config

- `route-fallbacks.md` lists `_commandcode` in the chains of `deepseek-flash-plan`, `deepseek-pro-plan`, `sol-plan`, `beam-plan`, `mimo-flash-plan` and `spark-plan`. All six were removed on 2026-11-13 (see e.2). **The doc is stale.**
- `route-fallbacks.md` gives `workhorse-last-resort` as "Sonnet 5.5 #medium (direct `_claude`) → fable-plan #low". Config is `_claude/claude-haiku-5-5#high` → `fable-plan#low` (3434-3436). **The doc is stale.**
- The `luna-plan` / `terra-plan` "orphan" notes (2716-2727) are stale: `terra-plan` has a consumer in `limited` (3654).

---

## b. How effort is expressed today

### b.1 Syntax and what it means

- `model#variant` pins a leaf to a variant, e.g. `openai-plan/gpt-6-luna#high` (2501) or `opencode-route/deepseek-flash-plan#low` (3002). A pinned ref sets `fixedVariant` on every leaf it reaches (MR:1039). **Any pin anywhere in a route's expansion makes that whole route advertise no variants** (see b.3).
- `variants: { from: to }` on a target is the remap (Schema:50-58). It is consumed at expansion (MR:1017-1021): the remap is kept **only if the door's model declares the target variant**; otherwise it is dropped silently.
- `defaultVariant` on a target sets the variant used when a route is selected bare (MR:1013-1016, Schema:50).
- Nothing else names effort: no `effort`, `low/medium/high` tier list, or overlay key exists in the config. `xhigh`, `max`, `ultra` and `fast` are never aliased in config; `ultra` and `fast` appear only inside catalog model ids (`deepseek/deepseek-v4-flash-fast`, `xiaomi/mimo-v2.6-pro-ultraspeed`, `kimi-k2.7-code-highspeed`, `glm-5.2-fast` on `_commandcode`, lines 1242-1243, 1299, 1257, 1278).

### b.2 Variants declared per provider and model (the only variant data in config)

| Provider / model | Declared variants (line) | Mechanism |
|---|---|---|
| `_claude` `claude-haiku-5-5`, `claude-sonnet-5-5`, `claude-opus-5-5`, `claude-fable-5-2` | `low`, `medium`, `high`, `xhigh`, `max` (284-326, 333-376, 382-425, 441-484) | Extended-thinking `budget_tokens` 4096 / 8192 / 16384 / 32768 / 65536 |
| `_vibe` `mistral-small-latest` | `none`, `high` (570-581) | `reasoning_effort` |
| `_vibe` `deepseek-v4-1-flash-1017` | `none`, `low`, `high`, `max` (602-623) | `reasoning_effort` |
| `_vibe` `zai-glm-5-3`, `zai-glm-5-3-flash` | `none`, `low`, `high`, `max` (638-659, 669-690) | `reasoning_effort` |
| `deepseek` (official, **disabled**) `deepseek-chat`, `deepseek-v4-pro`, `deepseek-flash` | `low`, `high`, `max` (836-852, 860-876, 887-903) | `reasoning_effort` |
| `meta` (**disabled**) and `_muse` `muse-spark-1.3-contributor`, `muse-wonder-1.0-contributor` | `minimal`, `low`, `medium`, `high`, `xhigh`, `max` (1015-1021, 1029-1035, 1084-1102) | `reasoning_effort` |
| `opendesign`, `cline-pass`, `_commandcode`, `opencode-go*`, `_openai.*`, `_antigravity`, `mistral` (overlay) | **none declared in config** | Catalog-provided. Not visible here. Per comments: DeepSeek on Go has `low/high/max` (2186-2187); ClinePass has no `medium` or `xhigh` (2174-2175); OpenDesign tops out at `high` (2174-2175, 2293) |

### b.3 How a caller's effort moves through a route (fork source)

1. **Advertised set.** `routeVariants(expanded)` (MR:1268-1279) returns `[]` if **any** leaf is pinned (`fixedVariant`, line 1269). Otherwise it is the intersection, over leaves, of each leaf's native variants ∪ its remap keys (1272-1278). So:
   - Every class route in a.1 advertises **no** variant, because each reaches a pinned leaf (e.g. `workhorse` reaches `deepseek-effort-rr` → `deepseek-flash-plan#high`).
   - Unpinned pools (`deepseek-flash-plan`, `sonnet-plan`, `luna-plan`, `sol-plan`, `mimo-flash-plan`, `argon-plan`, `beam-plan`, `spark-plan`, `opus-plan`, `gemini-hydrogen-plan`, `terra-plan`) advertise the intersection. The exact set **needs the live catalog**, which this audit cannot read. Open question Q1.
2. **Leaf selection at expansion.** For each door leaf (MR:958-978): a pinned variant is kept if the door can produce it natively or as `<model>-<variant>` (MR:967-968). A remap is preferred when it is producible (973). **If the requested variant is not producible, the leaf is dropped with `continue` (978), with no decision record.**
3. **Dispatch.** The resolver maps the caller's route variant through the leaf's remap; if none, it uses the same-named variant when the leaf has it natively; otherwise it loads the leaf at its default effort (MRes:1065-1073). A leaf's own pin wins (`target.variant ??`, MRes:1074).
4. **Unadvertised variant.** A caller that sends a variant the route did not advertise has no defined path in this audit. `VariantUnavailableError` exists (MRes:28-37, raised at 233-238 and 1433). Whether that path fires for a route-level variant is open question Q2.

**Answer to "does any pool expose low/medium/high as a tier to callers today?"** No. Tier routes pin each leaf to its own rung (`#low`, `#medium`, `#high`, `#xhigh`), and their advertised variant list is empty. The unpinned `*-plan` pools are the only places a caller could choose an effort, and only if the intersection contains the rung.

### b.4 Silent remaps and pins that change effort

| Where | What happens | Line(s) |
|---|---|---|
| `deepseek-flash-plan` Go and ClinePass legs | **`medium` → `high`** (the "DeepSeek #medium is not a real rung" case). A `#medium` caller gets `#high` on those doors. | 2285, 2289 |
| `deepseek-pro-plan` Go and ClinePass legs | **`medium` → `high`** on Go (2364) and ClinePass (2368). ClinePass also remaps **`low` → `high`** (2368), so a `#low` caller gets `#high` there. | 2364, 2368 |
| `deepseek-flash-plan` mistral leg | `low`→`low`, `medium`→`high`, `xhigh`→`max`, `max`→`max`. | 2305 |
| `sonnet-plan`, `opus-plan` Antigravity legs | `xhigh` and `max` → `high`. | 2572, 2596 |
| `gemini-hydrogen-plan` | `max` → `xhigh` (Hydrogen has a native xhigh tier). | 2544 |
| `luna-plan`, `sol-plan`, `deepseek-flash-plan`, `mimo-flash-plan`, `deepseek-pro-plan` OpenDesign legs | `xhigh`/`max` → `high`. Only if the OpenDesign model declares `high` (catalog). | 2470, 2683, 2293, 2372 |
| `glm-5-3-plan`, `glm-5-3-flash-plan` (defined only) | `xhigh` → `max`. GLM has no `medium`, so a `#medium` request prunes the leaf (978). | 2835, 2853 |
| `workhorse-weighted`, `summit`, `speed`, `specialist` | Fixed pins: `#high`, `#low`, `#medium`, `#xhigh`, `#high` on Spark and Hydrogen. Pins override whatever the caller asked for. | 3382-3384, 3436, 3480-3485, 3536-3538, 3586-3592 |

**Evidence for `medium`.** `deepseek-effort-rr` (2977-2982) records that `route_attempt` shows `deepseek-v4.1-flash` at `#high` for 66,113 attempts in 14 days and that `#medium` is not a real rung on the metered leaves. Its trial is therefore `#high` vs `#low` only (3001-3002).

**Evidence for silent pruning.** The mistral-small comment (557-569) records that an empty variant list on `_vibe` emptied `mistral-plan`'s variant set and broke pinned sessions ("Variant unavailable: low/high") until the variants were declared.

### b.5 Effort tiers and AA data as they stand

- There is no table of low/medium/high equivalents per model. Equivalents live in comments, for example "AA-match rule" (3334-3345) and "AA v5 cost per point" (3032-3033).
- The brief's "summit low = workhorse high" pattern does not exist as data. It is the nearest pin in each route, and each pin is decided by hand.

---

## c. Context limits

### c.1 Native limits declared in config

| Provider / model | `limit.context` | `limit.output` | `limit.input` | Line |
|---|---|---|---|---|
| `_claude` haiku/sonnet/opus/fable 5.x | 1,000,000 | 64,000 | — | 328-331, 377-380, 426-429, 485-488 |
| `_vibe` `mistral-vibe-cli-latest` | 262,144 | 32,000 | — | 544-547 (comment 530-540: "fork default 200000/32000" for models with no limit) |
| `_vibe` `deepseek-v4-1-flash-1017` | 1,048,576 | 128,000 | — | 598-601 |
| `deepseek` (disabled) `deepseek-chat`, `deepseek-v4-pro`, `deepseek-flash` | 1,048,576 | 393,216 | — | 832-835, 856-859, 883-886 |
| `openrouter` (disabled) minimax, laguna | 204,800; 131,072 | 131,072; 32,768 | — | 921-924, 928-931 |
| `groq` (disabled) qwen, gpt-oss-120b | 131,072 | 8,192; 16,384 | — | 944-947, 951-954 |
| Everything else | **not declared** (catalog) | | | |

Comments give catalog-native contexts for several models: gpt-6-luna and gpt-6.1-terra 1.05M (3614-3615, 3624), MiMo Flash 1,048,576 (3638), and Haiku 1M (3640-3644). These are not declared in the config and are not verified here.

### c.2 Route-level and leaf caps (and where they differ from native)

| Leaf (pool context) | Native (from above or comment) | Cap in config | Difference | Line |
|---|---|---|---|---|
| `luna-plan#medium` (via `limited`) | 1,050,000 (comment 3614) | **100,000** | Plan price cliff (3610-3613) | 3653 |
| `terra-plan#low` (via `limited`) | 1,050,000 (comment 3624) | **388,000** | Plan/policy ceiling, not model limit | 3654 |
| `haiku-plan#medium` (via `limited` and `small`) | 1,000,000 (328-331) | **388,000** | Policy ceiling | 3667 |
| `deepseek-flash-plan#low` (via `limited`) | 1,048,576 (833-886) | **1,000,000** | Slightly below native | 3658-3666 |
| `mimo-flash-plan` (via `limited`) | 1,048,576 (comment 3638) | 1,048,576 | Same | 3655-3656 |

Effective per-leaf ceiling: `min(model.limit.context, target contextLimit)` (MR:289-291; stored as `targetLimits`, 318-324). A cap set on a route or target is applied through the whole nested path, with the smallest value winning (MR:847-850, 1004-1010). A request larger than a leaf's ceiling skips that leaf at every level (MRes:932-933, MRoute:2795-2807).

### c.3 Output, input and cache

- **Output.** A route's `limit.output` is `min` over its leaves (MR:338). The route cannot set its own output cap.
- **Input.** Derived only when every leaf declares `limit.input` (MR:339-357). No model in the config declares `input`, so route input is undefined.
- **Cache.** No cache ceiling exists in the schema. `limit` has only `context` (Schema:61-70, 494-500). `cache_format` is per route or target (Schema:71, 461; MR:856, 1026) and is set nowhere in the config. **Strictest in/out/cache/total per model is not expressible today** beyond `context`.

---

## d. Agent pins and defaults

| Key | Value | Line |
|---|---|---|
| Default model (`model`) | `opencode-route/workhorse` | 254 |
| Small model (`small_model`) | `opencode-route/speed` | 260 |
| `build`, `general`, `plan` | no `model` key in the jsonc block or in `.opencode/agents/{build,general,plan}.md` frontmatter; they inherit the default | 1542 onward; `.opencode/agents/*.md` |
| `explore` | `opencode-route/scout` | 1754 |
| `classifier` | `opencode-route/scout` | 1757 |
| `classifier-complex` | `opencode-route/workhorse` | 1633 |
| `reasoner` | `opencode-route/specialist` | 1855 |
| `coder` | `opencode-route/workhorse` | 1947 |
| `compaction` | `threshold: 0.5` | 2043-2045 |

No agent pins a variant. No `.opencode/agents/*.md` file carries a `model:` key (grep). `explore`'s pin is explained at 1734-1752 (explore children were 716 of 747 on workhorse in the 7 days to 2026-11-13).

---

## e. Live door health (72 h) and Command Code

### e.1 Door health

Source: `route_usage` hours 72, per provider/model leaf. Failure rate = `failed / requests`. ≥90% is flagged. The tool does not give a failure reason, so a 100% rate can be a quota wall, a dead door, or a removed seat's residue.

**Totals:** 149,771 attempts, 38,832 failed. **36 of 50 leaves are at or above 90%.**

| Provider | Model | Requests | Failed | Rate | Flag / note |
|---|---|---|---|---|---|
| `_antigravity` | claude-opus-5-5 | 1 | 1 | 100% | **flag** |
| `_antigravity` | claude-sonnet-5-5 | 13 | 13 | 100% | **flag** (seat: workhorse-weighted 3384, sonnet-plan 2571) |
| `_antigravity` | gemini-3.8-flash | 54 | 54 | 100% | **flag**. Residue: no longer seated (replaced by Hydrogen, 3383) |
| `_antigravity` | gemini-4-argon | 295 | 101 | 34% | ok |
| `_antigravity` | gemini-4-hydrogen | 125 | 125 | 100% | **flag** (seats 3383, 3536, `gemini-hydrogen-plan`) |
| `_claude` | claude-fable-5-2 | 2,975 | 2,609 | 88% | near threshold, not flagged |
| `_claude` | claude-haiku-5-5 | 8,499 | 5,581 | 66% | ok, but heaviest Claude seat |
| `_claude` | claude-opus-5-5 | 108 | 106 | 98% | **flag** |
| `_claude` | claude-sonnet-5-5 | 16,099 | 14,588 | 91% | **flag** (near 90) |
| `_commandcode` | deepseek/deepseek-v4.1-flash | 717 | 717 | 100% | **flag**. Seat removed 2026-11-13 |
| `_commandcode` | gpt-6.1-sol | 243 | 243 | 100% | **flag**. Seat removed 2026-11-13 |
| `_commandcode` | reflection/beam:free | 513 | 513 | 100% | **flag**. Seat removed 2026-11-13 |
| `_commandcode` | xiaomi/mimo-v2.6-flash | 4 | 4 | 100% | **flag**. Seat removed 2026-11-13 |
| `_muse` | muse-spark-1.3-contributor | 256 | 256 | 100% | **flag** (seats 3382, 3586, 3640-3642 pool) |
| `_muse` | muse-wonder-1.0-contributor | 1 | 1 | 100% | **flag** |
| `_openai` (retired id) | gpt-6-luna | 28 | 28 | 100% | **flag**. Retired id, residue |
| `_openai` (retired id) | gpt-6.1-sol | 63 | 3 | 5% | ok |
| `_openai` (retired id) | gpt-6.1-terra | 17,858 | 637 | 4% | ok |
| `_openai.alt` | gpt-6-luna | 339 | 26 | 8% | ok |
| `_openai.alt` | gpt-6.1-sol | 18 | 8 | 44% | ok |
| `_openai.alt` | gpt-6.1-terra | 3,404 | 2,783 | 82% | near threshold, not flagged |
| `_openai.main` | gpt-6-luna | 76,058 | 113 | 0.1% | ok (main volume door) |
| `_openai.main` | gpt-6.1-sol | 936 | 71 | 8% | ok |
| `_openai.main` | gpt-6.1-terra | 2,881 | 2,825 | 98% | **flag** |
| `_vibe` | deepseek-v4-1-flash-1017 | 1,097 | 1,096 | 100% | **flag** (mistral-plan vibe leg) |
| `claude-code` (retired id) | claude-haiku-5-5 | 163 | 111 | 68% | ok, residue |
| `cline-pass` | deepseek-v4.1-flash | 654 | 654 | 100% | **flag** |
| `mistral` | deepseek-v4-1-flash-1017 | 141 | 141 | 100% | **flag** (API reserve leg) |
| `openai.alternate` (retired id) | gpt-6-luna | 748 | 245 | 33% | ok, residue |
| `openai.alternate` (retired id) | gpt-6.1-sol | 108 | 10 | 9% | ok, residue |
| `openai.alternate` (retired id) | gpt-6.1-terra | 19 | 19 | 100% | **flag**, residue |
| `openai.main` (retired id) | gpt-6-luna | 7 | 7 | 100% | **flag**, residue |
| `openai.main` (retired id) | gpt-6.1-sol | 8 | 8 | 100% | **flag**, residue |
| `openai.main` (retired id) | gpt-6.1-terra | 25 | 25 | 100% | **flag**, residue |
| `opencode-go.developing-today-0` | claude-haiku-5-5 | 518 | 518 | 100% | **flag** |
| `opencode-go.developing-today-0` | deepseek-v4.1-flash | 116 | 116 | 100% | **flag** |
| `opencode-go.developing-today-0` | gemini-4-argon | 10 | 10 | 100% | **flag** |
| `opencode-go.developing-today-0` | muse-spark-1.3-contributor | 10 | 10 | 100% | **flag** |
| `opencode-go.drewrypope` | claude-haiku-5-5 | 504 | 504 | 100% | **flag** |
| `opencode-go.drewrypope` | deepseek-v4.1-flash | 112 | 112 | 100% | **flag** |
| `opencode-go.drewrypope` | gemini-4-argon | 2 | 2 | 100% | **flag** |
| `opencode-go.drewrypope` | muse-spark-1.3-contributor | 10 | 10 | 100% | **flag** |
| `opencode-go.popesdrew` | claude-haiku-5-5 | 396 | 396 | 100% | **flag** |
| `opencode-go.popesdrew` | deepseek-v4.1-flash | 288 | 288 | 100% | **flag** |
| `opencode-go.popesdrew` | gemini-4-argon | 8 | 8 | 100% | **flag** |
| `opencode-go.popesdrew` | muse-spark-1.3-contributor | 10 | 10 | 100% | **flag** |
| `opendesign` | claude-haiku-5.5 | 340 | 340 | 100% | **flag** (no seat now: haiku-plan lost OpenDesign 2026-11-11, 2405-2406; residue) |
| `opendesign` | deepseek-v4.1-flash | 12,941 | 2,738 | 21% | ok. Highest-volume non-OpenAI door |
| `opendesign` | gpt-6.1-sol | 46 | 46 | 100% | **flag** |
| `opendesign` | mimo-v2.6-flash | 2 | 2 | 100% | **flag** |

Pattern to note: every `opencode-go.*` row (all three workspaces, all four models) is at 100%. Config notes say the Go weekly windows are walled until 2026-11-15 (2774-2775, 3324-3326). If so, these are quota walls, not dead doors. The tool cannot separate the two.

### e.2 Command Code seats still present

After PR #9401 (code#9397, 2026-11-13):

- **Seats in any route: none.** The six removals are: `deepseek-flash-plan` (2167), `deepseek-pro-plan` (2350), `sol-plan` (2649), `beam-plan` (2771), `mimo-flash-plan` (2794) and `spark-plan` (2604). No route references `_commandcode/*` (grep).
- **Provider still ON.** `_commandcode` is not in `disabled_providers` (210-224). Its definition and about 110 catalog models stay (1212-1539). The 2026-11-06 note says it rejoins automatically on a measured balance increase (176-183).
- **Plan coverage.** Sol lost plan coverage on 2026-11-09 (403 `MODEL_NOT_IN_PLAN`, 2649-2652). The free Beam 1.0 tier ended on 2026-11-13 (403 on every call, 2771-2773). Monthly credits: $9.95 of $10 used, "insufficient credits" (2168-2170, 2351-2353, 2796-2797).
- **Live traffic, 72 h (before the removal):** 717 + 243 + 513 + 4 = **1,477 attempts, all failed** (see e.1).
- **Catalog-only Claude models on Command Code** (Sonnet, Opus, Fable, Haiku) hold no seat (2408-2409). Route `fable-plan` is on `_claude` only.

### e.3 Retired and stale ids still in traffic

- `_openai` (28 attempts, 100% failed): the id was renamed to `_openai.main` on 2026-11-10 (lines 758-762).
- `openai.alternate`, `openai.main` (retired id forms, 748 + 7 + 8 + 25 + 108 + 19 attempts): removed definitions (716-720, 745-749).
- `claude-code/claude-haiku-5-5` (163 attempts, 68% failed): renamed to `_claude` (274-281).
- `opendesign/claude-haiku-5.5` (340 attempts, 100% failed): no seat any more (2405-2406).

---

## f. Gaps against the brief (what does not exist yet)

**Low / medium / high per pool**

- No tier pool exposes low/medium/high as a caller choice. Every tier route pins its leaves; the advertised variant list is `[]` whenever a pin is reached (MR:1269).
- No table of low / medium / high equivalents per model. Equivalents are decided in comments (for example 3334-3345).
- No low or high rung defined for `opendesign`, `cline-pass`, `_commandcode`, `opencode-go*`, `_openai.*` or the Antigravity doors in config. Their variants are catalog-provided and unverified here.
- `#medium` is a false rung on DeepSeek Go and ClinePass (2285, 2289, 2364, 2368). ClinePass also remaps `low` to `high` on Pro (2368).

**Overlays**

- No overlay mechanism exists. The only way to change an effort is to edit the leaf, the route, or the pin, by hand, in the one file.
- No "set effort for this model", "remove model-effort", or "replace model-effort" keys exist.

**Aliases**

- `xhigh`, `max`, `ultra` and `fast` are not aliased to `high`. They are real rungs on Claude, Muse and DeepSeek, and remapped per leaf elsewhere (b.4). The brief's "until implemented, make xhigh / max / ultra / fast point to high" has no config form.

**Context-tier limits**

- Only `limited` (a.4) uses per-target context caps. No route-level `limit.context` is set.
- No per-model strictest in / out / cache / total limit. Only `context` exists in the schema, plus output as a min over leaves (c.3).
- `cache_format` is unused.

**Other gaps the brief depends on**

- No calendar or holiday windows (opencode#821; config 3698-3717).
- No granular stats store (brief §6). The route_usage tool is the only per-attempt source here.
- `route-fallbacks.md` is stale (a.7), and the guard (`bin/check-opencode-route-rules.mjs`) does not check remap targets against catalogs.

---

## Open questions (Q) with the default used

- **Q1.** Which variants does each unpinned pool advertise at runtime? *Default:* not computed; needs a live `/route` or catalog dump. The JSON leaves the variant lists blank for those doors.
- **Q2.** When a caller sends a variant that a route did not advertise, which path fires (silent default effort vs `VariantUnavailableError`)? *Default:* recorded as unverified; MRes:1065-1073 suggests the bare default effort, with no error.
- **Q3.** Are the remaps to `high` and `max` on OpenDesign and ClinePass live (the catalog declares those variants)? A remap is dropped silently if the target variant is absent (MR:1017-1021). *Default:* treated as live, per comments 2174-2175.
- **Q4.** Are the 100% failure rates on the Go workspaces, Muse and Antigravity Hydrogen quota walls or dead doors? *Default:* quota walls for Go (per config comments 3324-3326), unknown for Muse and Hydrogen. Route_usage has no reason field.
- **Q5.** Should `route-fallbacks.md` be regenerated from config (it is stale on 3 points, a.7)? *Default:* left unchanged; this audit writes only its own files.
