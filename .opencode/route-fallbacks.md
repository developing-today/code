# Route fallback layout (Drew, 2026-11-13)

The complete fallback layout for every route in `.opencode/opencode.jsonc`.
The class edges are wired as the FINAL targets of each class route and are
guarded by `bin/check-opencode-route-rules.mjs` (`CLASS_FALLBACKS` there must
match this table and the config; the guard fails on drift).

## Semantics (Drew 2026-11-13)

1. **Regular chain first.** A pool's own regular chain always runs first and
   completely — ordered targets, weighted draws, and door failover inside the
   *-plan pools included. Class fallback never interleaves with or
   short-circuits the regular chain.
2. **Fallback is a second pass.** Fallback is a per-pool property and engages
   only after that pool's full regular chain is exhausted (all legs failed or
   quota-walled; a 100%-walled leg is never attempted — it counts as
   exhausted immediately). The fallback class brings its own seats and
   purpose.
3. **Walked down the chain, no repeats.** Entering a fallback class runs THAT
   class's full regular chain first, then its own fallback if it too
   exhausts — subject to a per-call visited set: a class already tried in
   this call is skipped, never re-entered. The class set is finite and each
   escalation consumes one, so every walk terminates at **specialist**, the
   sink. (Mechanism: the fork's route expander carries the expansion path
   and prunes re-entry into a route already on it, so the legal pairs
   workhorse → scout / scout → workhorse cannot loop.)

## Class routes (all confirmed by Drew; scout and background edges ruled 2026-11-14)

| Route | Regular chain (in order) | Class fallback | Why |
| --- | --- | --- | --- |
| small | limited (the per-member pool) | → speed | Light tier escalates to the latency tier. |
| speed | Spark #xhigh → deepseek-flash-plan #high → gemini-hydrogen-plan #high → haiku-plan #high | → workhorse → summit → specialist | Cheaper / more-available first, quality next, sink last. |
| workhorse | workhorse-ordered → workhorse-primary (empty stage) → workhorse-weighted → workhorse-last-resort | → scout → specialist | Workhorse's failure mode is quota exhaustion; scout diversifies onto different doors (free Beam via CommandCode, DeepSeek #low, Haiku #low). Scout-first was main's recommendation, wired 2026-11-13 pending Drew's pick. |
| summit | fable-plan #medium → opus-plan #high → argon-plan #high → Hydrogen #high (direct) → Wonder #xhigh (direct) → sol-plan #xhigh | → specialist | Confirmed: the deliberate tier falls back to the hard-work tier. |
| specialist | fable-plan #low → sonnet-plan #high → argon-plan #high → spark-plan #xhigh → sol-plan #medium | — (SINK) | No class fallback out of specialist; if every leg is down the call errors. |
| scout | beam-plan → longcat-plan → iceman-plan → deepseek-flash-plan #low → haiku-plan #low → inkling-plan | → background → workhorse | Ruled by Drew 2026-11-14: routine work that exhausts the free/cheap seats escalates to the patient tier first, then the standard tier. |
| background | MiMo Flash / DeepSeek Flash #high, peak-window aware (see config) → Zen free shelf: flash-free-plan → minimax-flash-plan → glm-5-3-flash-zen-plan → flint-plan → big-pickle-plan → nemotron-vl-plan → nemotron-lightning-plan | → workhorse | Ruled by Drew 2026-11-14: patient queue work escalates to the standard tier. |

## Workhorse stages and trial routes (no own class edges — escalation is workhorse's)

| Route | Targets | Fallback |
| --- | --- | --- |
| workhorse-ordered | deepseek-effort-rr | Inherits workhorse (stage 1 of its chain). |
| workhorse-primary | — (deliberately empty; contributes nothing) | Inherits workhorse (stage 2). |
| workhorse-weighted | luna-effort-rr w10, Spark contributor #high w10, Hydrogen #low w10, Sonnet 5.5 #medium (direct _antigravity) w10 | Inherits workhorse (stage 3). |
| workhorse-last-resort | Sonnet 5.5 #medium (direct _claude) → fable-plan #low | Inherits workhorse (stage 4, last before the class fallback). |
| deepseek-effort-rr | deepseek-flash-plan #high w3 / #low w1 (weighted trial, PR #9381) | Inherits workhorse via workhorse-ordered. |
| luna-effort-rr | openai-plan Luna #medium / #high (round-robin trial, PR #9018) | Inherits its consumers (workhorse-weighted). |
| limited | Luna @100k #medium → Terra @388k #low → MiMo/DeepSeek-small peak-aware → Haiku @388k #medium | Inherits small. |
| deepseek-flash | deepseek-flash-plan (pass-through wrapper) | Inherits its consumers. |

## *-plan pools (no class edges — fallback is internal door failover)

Class escalation is inherited from the consuming class route above; a plan
never escalates on its own. "Doors" are the providers the pool fails over
between, in the pool's selection order.

| Pool | Doors (selection) | Note |
| --- | --- | --- |
| deepseek-flash-plan | OpenCode Go (3 workspaces) → ClinePass → OpenDesign Flash Eco → CommandCode → Mistral plan (round-robin) | The fleet's main DeepSeek pool. |
| deepseek-pro-plan | OpenCode Go → ClinePass → OpenDesign → CommandCode (round-robin) | Pro sibling of the flash pool. |
| luna-plan | openai-plan → OpenDesign → OpenCode Go (round-robin) | Legs present different contexts (200k vs 1.05M native). |
| terra-plan | openai-plan only (round-robin over its 2 doors) | Single-plan pool; door failover is openai-plan's. |
| sol-plan | openai-plan → OpenDesign → CommandCode (round-robin) | |
| fable-plan | _claude only | **Single door — no internal failover.** If _claude is down, consumers fall through to their next seat. |
| haiku-plan | _claude only | **Single door — no internal failover** (same fall-through). |
| sonnet-plan | _claude → Antigravity (round-robin) | |
| opus-plan | _claude → Antigravity (ordered) | |
| spark-plan | OpenCode Go → CommandCode → ClinePass → _muse (ordered) | Meta Spark across four doors. |
| argon-plan | OpenCode Go → Antigravity (round-robin) | |
| gemini-hydrogen-plan | Antigravity only | **Single door — no internal failover.** |
| beam-plan | OpenCode Go (beam-free) → OpenCode Zen (beam-free) (round-robin) | The free-tier pool behind scout. (CommandCode door removed 2026-11-13, #9397; cat-jam-free is the same model — deliberately not seated. Zen door at full weight 3 since 2026-11-14: Drew’s limited-context demotion was posed as a question; models.dev lists identical context on both doors, 1,048,576.) |
| longcat-plan | OpenCode Go (longcat-2.5-free) → OpenCode Zen (longcat-2.5-free) (round-robin) | Free LongCat 2.5 pool behind scout (2026-11-14, probe-verified 3/3 workspaces). No AA listing. |
| iceman-plan | OpenCode Go (iceman-contributor-free) → OpenCode Zen (iceman-free) (round-robin) | Free Iceman pool behind scout (2026-11-14, probe-verified 3/3 workspaces). Stealth model, identity unclaimed; no AA listing. |
| mimo-flash-plan | OpenCode Go → ClinePass → OpenDesign → CommandCode → OpenCode Zen (mimo-v2.6-flash-free) (round-robin) | |
| glm-5-3-plan | _vibe (Mistral) only | **Single door — no internal failover.** |
| glm-5-3-flash-plan | _vibe (Mistral) only | **Single door — no internal failover.** |
| flash-free-plan | OpenCode Zen (deepseek-v4.1-flash-free) (round-robin) | Zen's free DeepSeek Flash (AA v5 II 33 max) behind background (2026-11-14). Own pool: deepseek-flash-plan's leaves carry variant remaps the free door is not verified against. |
| inkling-plan | OpenCode Zen (inkling-2-small-free) (round-robin) | Free Inkling 2 Small (AA v5 II 31) behind scout (2026-11-14). |
| glm-5-3-flash-zen-plan | OpenCode Zen (glm-5.3-flash-free) (round-robin) | Free GLM 5.3 Flash (AA v5 II 28, ~48 tok/s) behind background (2026-11-14). Named distinctly from the _vibe glm-5-3-flash-plan above. |
| minimax-flash-plan | OpenCode Zen (minimax-m3.1-flash-free) (round-robin) | Free MiniMax M3.1 Flash (AA v5 II 29) behind background (2026-11-14). Rarely abstains — bulk work only. |
| flint-plan | OpenCode Zen (flint-40b-free) (round-robin) | Free Flint 40B coding model behind background (2026-11-14). No AA listing. |
| big-pickle-plan | OpenCode Zen (big-pickle) (round-robin) | Zen stealth free model behind background (2026-11-14). Rotating backend — no benchmark attaches. |
| nemotron-vl-plan | OpenCode Zen (nemotron-3.5-super-vl-free) (round-robin) | Free Nemotron VL (AA v5 II 13.6) behind background (2026-11-14). |
| nemotron-lightning-plan | OpenCode Zen (nemotron-3.5-lightning-free) (round-robin) | Free Nemotron Lightning (AA v5 II 5.5) — last free leg in background (2026-11-14). |
| systemone | OpenCode Zen (jev-1.13-free) | **Decision models — NOT a chat pool.** System-1 typed-question endpoint; never seated in a chat tier (2026-11-14). |

## Provider routes (door sets, expanded per model)

| Provider route | Doors |
| --- | --- |
| openai-plan | _openai.alt, _openai.main (server-side round-robin) |
| mistral-plan | _vibe, mistral |
| opencode-go-plans | opencode-go.drewrypope, opencode-go.popesdrew, opencode-go.developing-today-0 |
| zen-plans | opencode.drewrypope, opencode.popesdrew, opencode.developing-today-0 |

## Guard

`bin/check-opencode-route-rules.mjs` enforces: (a) each class route's targets
end with exactly its declared fallback chain (drift fails); (b) every class
can reach specialist through the fallback graph; (c) a simulated escalation
walk from every class terminates at specialist without revisiting a class;
(d) the composition graph (all route→route edges EXCEPT the declared
fallback pairs) is acyclic — a composition cycle is reported with its path.
