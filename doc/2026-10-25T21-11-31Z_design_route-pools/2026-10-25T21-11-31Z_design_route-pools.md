# Route pools: tier pools over per-model plan pools

- Status: accepted (built and verified in the same change)
- Date: 2026-10-25T21-11-31Z
- Roadmap item: none
- Issues: none (Drew's direction, 2026-10-25)

## Intent

Drew asked for the fleet's model routing to be rebuilt as composable route
pools (`experimental.model_routes` in `.opencode/opencode.jsonc`):

- Tier pools organized by the three axes he switches on — cost, speed,
  effective smartness — instead of the ad-hoc `cheap-pool` / `fast` /
  `balanced` / `deep` / `free` set, which named targets on providers that are
  disabled fleet-wide (Cloudflare gateway, Groq, OpenRouter, direct Google)
  and used the pay-as-you-go DeepSeek provider as a routine member.
- Per-model pools that join one model across every provider serving it, with
  tier pools including those pools as single nested items.
- Per-target effort (variant) settings inside tiers: in the standard tier,
  Luna runs at max, Haiku at high, DeepSeek at medium or high.
- The direct DeepSeek provider (`deepseek`, api.deepseek.com) is
  pay-as-you-go and a reserve only: a DeepSeek plan pool and a PAYG pool
  combined in an ordered parent, so plan burns until every plan route fails
  or is out of quota, then it cuts over.
- A way to deprioritize a single target in a pool without weighting all the
  rest.
- The named agents (coder, reasoner, classifier, classifier-complex) point
  at pools instead of hard model pins, so a single exhausted provider (the
  Claude route was recently fully out of quota) no longer strands an agent.

## Design

All pools live in `experimental.model_routes` with `autonomy: "rules"` (the
level at which selection modes, weights, budgets, and provider rate-limit
holds apply). Two layers:

### Per-model pools (one model, every plan provider that serves it)

| Pool                  | Members (round-robin unless noted)                                                                                                           |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `deepseek-flash-plan` | cline-pass/deepseek-v4.1-flash, opendesign/deepseek-v4.1-flash, opendesign/deepseek-v4-flash                                                 |
| `deepseek-flash`      | ordered: `opencode-route/deepseek-flash-plan`, then deepseek/deepseek-chat (PAYG reserve); the plan group carries weight 100 (see Decisions) |
| `deepseek-pro-plan`   | cline-pass/deepseek-v4-pro, opendesign/deepseek-v4-pro                                                                                       |
| `haiku-plan`          | claude-code/claude-haiku-5-5, opendesign/claude-haiku-5.5                                                                                    |
| `luna-plan`           | openai/gpt-6-luna, opendesign/gpt-6-luna, opencode-go.lab/gpt-5.6-luna, opencode-go.personal/gpt-5.6-luna                                    |
| `glm-flash-plan`      | cline-pass/glm-5.3-flash, opendesign/glm-5.3-flash, opendesign/glm-5.3-flashx                                                                |
| `gemini-flash-plan`   | antigravity-ls/gemini-3.8-flash                                                                                                              |
| `sonnet-plan`         | antigravity-ls/claude-sonnet-5-5                                                                                                             |
| `opus-plan`           | antigravity-ls/claude-opus-5-5 (ordered; single member)                                                                                      |
| `sol-plan`            | openai/gpt-6.1-sol, opencode-go.lab/gpt-5.6-sol, opencode-go.personal/gpt-5.6-sol                                                            |

Membership rules: plan routes only; a provider disabled in
`disabled_providers` contributes nothing; a model id is listed only where
that provider actually serves it (e.g. OpenCode Go serves Luna as
`gpt-5.6-luna`; `deepseek-v4-flash` is not a valid Go model id, so Go is not
a DeepSeek Flash member). The PAYG tail of `deepseek-flash` is the direct
provider's `deepseek-chat`: the direct API config defines no separate Flash
id, so the reserve door is its general chat model.

### Tier pools (nested per-model pools as single items)

Effort is expressed as the variant suffix on the nested reference
(`opencode-route/luna-plan#max`); the variant is inherited into the nested
route and each leaf target's own variant remap (`defaultVariant` /
`variants`) applies from there. There is no `effort` field in the schema;
the variant is the mechanism.

| Tier         | Axes                                      | Selection                                                               | Members                                                                  |
| ------------ | ----------------------------------------- | ----------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `scout`      | routine: cheapest + fastest, smart enough | round-robin                                                             | deepseek-flash#medium, glm-flash-plan#medium, haiku-plan                 |
| `workhorse`  | standard: fast and smart — most tasks     | round-robin                                                             | deepseek-flash#high, luna-plan#max, haiku-plan#high, meta/muse-spark-1.3 |
| `specialist` | hard: quality first                       | ordered (registry family order: Claude, then OpenAI, then DeepSeek Pro) | sonnet-plan#high, luna-plan#max, sol-plan#medium, deepseek-pro-plan#high |
| `summit`     | very complex design only, deliberate      | ordered                                                                 | opus-plan#high, openai/gpt-6.1-sol#high, meta/muse-wonder-1.0            |

### Agent defaults

- `classifier` → `opencode-route/scout`
- `classifier-complex` → `opencode-route/workhorse`
- `coder` → `opencode-route/workhorse`
- `reasoner` → `opencode-route/specialist`

## Verified in the fork (no fork change was needed)

Checked against `developing-today/opencode` source, not documentation:

- **Nesting works for selection, recursively and lazily.**
  `packages/core/src/model-route.ts` `nextTarget` walks a node tree: a
  nested route is a child node with its own selection and weights, only
  groups on the chosen path consume their round-robin/weighted draw, and
  failover falls through the remaining targets in ranked order.
- **Effort remapping is the variant mechanism.**
  `packages/schema/src/config/model-routes.ts`: a target is
  `provider/model#variant` or an object with `defaultVariant` and a
  `variants` remap (route-level variant → this target's variant).
- **Demote-a-single-target already exists.** Round-robin is _smooth
  weighted round robin_ (equal weights are ordinary round robin), weights
  rank candidates in every selection mode, and targets without a weight
  count as 1. Setting one entry in a route's `weights` — e.g.
  `{ "some/target": 0.1 }` — deprioritizes just that target; the rest keep
  the default. On top of that, the engine demotes a target automatically
  after repeated quota errors (weight × 0.01) and there are user
  preference overrides (`route_adjust`). No new field was added: an
  OpenChamber-side `demote`/`priority` key would be refused by its
  validator as unknown, and adding it upstream would duplicate the weights
  mechanism.
- **Quota/failure exclusion is automatic.** A failure the runner classifies
  as quota (`QuotaExceeded`, or a captured rate-limit snapshot in
  `packages/core/src/session/runner/step.ts`) cools the target for the
  provider's retry-after when given, else the route health
  `quotaCooldownMs` (default 15 min); ordinary failures cool for
  `cooldownMs`. Budgets' `softLimit` passes a target over before the
  provider starts refusing it.
- **OpenChamber's pool validator accepts this whole design unchanged**
  (nested references, `#variant` on a reference, `weights` keyed by
  `opencode-route/<id>`). Verified by running
  `validateModelRoutes` from
  `packages/web/server/lib/routing/model-routes.js` against the new route
  set: no problems reported.

## Decisions

- **Weight 100 on the plan group inside `deepseek-flash`.** In an ordered
  route, candidates are ranked by effective weight; a quota-demoted plan
  leaf (×0.01) drags its group's average weight down. Weight 100 keeps the
  plan group ahead of the PAYG leaf (weight 1) even when every plan leaf
  is demoted (100 × 0.01 = 1, a tie, and ties keep configured order),
  while a plan group with no ready leaf still falls through to PAYG —
  exactly Drew's cutover rule.
- **commandcode is not a pool member anywhere.** The registry records its
  plan (Go plan) as having no API access; sessions must not fail over to
  it until Drew upgrades the plan tier. Its large model catalog joins the
  per-model pools by config edit only, the moment that changes.
- **`openai.alternate` is not a member yet.** The second ChatGPT account
  has no credential in the store (its device-code sign-in is still
  pending), so its models cannot serve. Add `openai.alternate/gpt-6-luna`
  and `openai.alternate/gpt-6.1-sol` to `luna-plan` / `sol-plan` when the
  sign-in lands.
- **Old route ids are removed, not aliased**: `cheap-pool`, `fast`,
  `balanced`, `deep`, `free`. Nothing in the repo's instructions, commands,
  or agents references them. Sessions or external state pinned to
  `opencode-route/fast` (etc.) must be re-pinned to a tier pool when this
  config deploys.
- **Tier names** (`scout`, `workhorse`, `specialist`, `summit`) name the
  kind of work, with the axes spelled out in each pool's `name` field and
  in the tier table above, rather than reusing light/fast/heavy.

## Alternatives considered

- **A new `demote` field in the fork schema + OpenChamber validator.**
  Rejected: weights already express it in every selection mode (verified
  above); a second spelling would need schema, engine, validator, client,
  and editor changes to duplicate existing behavior.
- **Keeping the old route ids as aliases to the new tiers.** Rejected:
  `fast` and `balanced` had PAYG DeepSeek as a routine member and `free`
  pointed at a disabled provider; silent aliasing would hide the
  membership change behind familiar names.
- **Ordered tiers everywhere.** Rejected for `scout`/`workhorse`:
  round-robin spreads burn across plan pools (the measurement-gate
  doctrine is to spread early, not ride one pool into its wall); ordered
  is kept where quality order matters (`specialist`, `summit`) and for
  the plan→PAYG composition.

## Consequences

- Agent sessions survive a single provider's quota wall: every named
  agent now draws on several plan pools, and failing or quota-exhausted
  targets cool down and are skipped automatically.
- The DeepSeek Flash path spends plan quota first across three providers
  and reaches the PAYG reserve only when no plan leaf is ready — the one
  deliberate exception to the plan-only rule, at Drew's direction.
- Adding a provider or an account later is a one-line membership edit in
  the per-model pool; tiers and agents do not change.

## Open questions

- **antigravity-ls membership.** The provider-quota registry (2026-10-24)
  records antigravity-ls as disabled on the sandbox by Drew's order, but
  the reconciled config (PR #904) does not list it in
  `disabled_providers`, and it is the only configured door to Sonnet and
  Opus. These pools include it, following the fresher config; if the
  disable still stands, remove its members and `sonnet-plan` /
  `opus-plan` / `gemini-flash-plan` until it is re-enabled. Settled by:
  Drew, or the scratch-server provider list at deploy time.

## 2026-10-25T21-45-00Z Discovery: scratch-server verification results

The config in this change was loaded by a scratch `opencode serve`
(port 4199, scratch XDG homes, its own password; the live server and the
deployed config file untouched), with the worktree as the location:

- `/api/config` resolves all 14 routes; targets parse into structured
  refs (nested members as `opencode-route/<id>`, tier variants carried:
  workhorse = deepseek-flash#high, luna-plan#max, haiku-plan#high,
  meta/muse-spark-1.3). All 18 disabled providers appear as
  `provider.use` deny policies.
- `/api/provider` lists `antigravity-ls` as **enabled** under this
  config — the server agrees with the reconciled config, not with the
  registry's 2026-10-24 disable note. Its pool membership stands; the
  registry entry is the stale one.
- The four named agents resolve to their pools in the parsed config:
  coder → workhorse, reasoner → specialist, classifier → scout,
  classifier-complex → workhorse. `opencode-go.lab` resolves as a
  provider instance of `opencode-go`.
- Not verified: leaf-level live requests (the scratch home holds no
  credentials), so per-target catalog resolution is proven for config
  and provider layers, not by an end-to-end model call.
- **commandcode plan tier.** Settled by Drew upgrading the plan; then its
  models join the per-model pools (see Decisions).
