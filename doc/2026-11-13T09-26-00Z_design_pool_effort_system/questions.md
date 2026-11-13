# Questions log — pool & effort system design mill (code#9399, queue item 15864)

Detached-operation rule: the summit session never stalls on a question. Every
question is recorded here **with the default chosen**, and work continues on
that default. Entries marked **RULING** are Drew's alone to settle (they are
batched to the orchestrator with options and a recommendation); everything
else is decided in the mill and can be overturned by a later ruling.

Format: `Q<n>` · date (CT) · status (`open-default` / `ruled` / `withdrawn`) ·
question · options · default chosen · why · where the default is applied.

---

## Q1 — 2026-11-13 06:05 CT — open-default — starter issue was closed by automation

**Observed:** code#9399 was labeled `status:done` and closed by
`developing-today[bot]` at 2026-11-13T11:47:33Z (two minutes after the
supplement comment; twelve minutes before this session launched). No human
closed it. The EPIC GUARD says the starter issue holds work in itself and is
never closed merely because subtasks complete.

**Options:** (a) leave it closed and comment on the closed issue; (b) reopen
it once, say why in a comment, leave the bot-owned `status:done` label alone,
and not fight the bot if it re-closes; (c) reopen and also remove the label.

**Default chosen:** (b). Reopening restores the state the standing rule
describes and is trivially reversible; the label is the bot's projection of
some upstream status (likely the queue/Kaneo task state of the failed first
launch) and editing it could feed back into systems this session does not
own. If the bot closes it again, this session keeps commenting on the closed
issue and flags the sync for the orchestrator instead of reopening in a loop.

**Applied:** issue comment on code#9399 at reopen time.

## Q2 — 2026-11-13 06:05 CT — open-default — "clef jev" phrase

**Status of the question:** the launch packet's supplement decodes the phrase
(clef and jev are decision-API models on the `v1/systemone` contract), so the
brief §1 phrase is no longer undecoded. Residual reading question: brief §1
says "wire up clef jev opencodego free then paid then cloudflare etc."

**Default chosen:** read it as *"wire up the decision models (clef, jev, the
OpenCode Go free ones), free first, then paid, then Cloudflare, etc."* — i.e.
the provider order is the **decision pool's** order (supplement §2: free →
plan → PAYG, Cloudflare included-usage burned first among metered options),
and it also informs the category-pool order in brief §1 only where a category
is served by a decision model. The statement inventory maps the phrase to the
decision-pool design, not to a category name.

## Q3 — 2026-11-13 06:40 CT — open-default — Cloudflare "$10 of included usage"

**Observed (R2 research):** the Workers paid plan's Workers AI allowance is
10,000 neurons/day (≈ $0.11/day at overage rates), not a $10 monthly credit;
paid plans have no hard stop at the allowance. Drew's wording was "$10 (or
whatever)".

**Default chosen:** the decision executor keeps its own per-UTC-day neuron
counter against the real included allowance (configurable, default 10,000
neurons/day), burns it first among metered options, and disengages at the
cap; Cloudflare's 429/overage is never the stop signal. If Drew meant a
separate dollar budget (e.g. a $10/month overage ceiling), it becomes a
second configurable cap on the same counter.

## Q4 — 2026-11-13 06:40 CT — open-default — "Luna 100k" in the decision chain

**Default chosen:** Luna 100k = the existing `limited` pool's first leg
(openai-plan gpt-6-luna #medium with the @100k policy cap, PR #9384), pinned
directly as the decision chain's last resort — never through `small` or
`limited` as a whole (supplement §4: "don't use small").

## Q5 — 2026-11-13 06:55 CT — open-default — who serves Jev (supplement §0 vs the code)

**Observed (V1 decision audit, baseline/v1-decisions-auto.md):** nothing in
the fork ties the systemone path to `_commandcode`. Jev is served as
TypeSafe `jev-latest`, OpenCode Zen `jev-1.13-free` / `jev-1.13`, OpenRouter
`typesafe/jev-1.13`, and Vercel `typesafe-ai/jev`. A live probe of Zen's
`/v1/systemone` with `jev-1.13-free` returned 200 with a typed answer
(2026-11-13). Clef (the Cloudflare Workers AI path behind the fork's
façade) was ruled won't-do in opencode#1651 (2026-11-04).

**Default chosen:** design on the code's facts. Jev's free door is OpenCode
Zen's promo (the "opencodego free" in brief §1 most plausibly means this),
followed by Jev's paid doors (TypeSafe / Zen-key / OpenRouter / Vercel), then
Cloudflare. Command Code is not a Jev door in V1 and is dead anyway. Clef is
recorded as **reopen-candidate**, not revived: Drew's supplement names it
(and the Cloudflare included-usage burn), so the V2 design keeps a Clef/
Workers-AI leg behind a flag, and the decision on reviving opencode#1651 is
batched as a **RULING** for Drew with the recommendation "revive as a
budget-capped leg only after the labelled cost-per-correct comparison".

## Q6 — 2026-11-13 06:55 CT — open-default — OpenChamber category `research` pins effort `max`

**Default chosen:** per brief §4 ("until implemented, make xhigh / max /
ultra / fast all point to high"), V2 treats `max` as an alias of `high`; the
category config is left as is, and the alias is applied in the effort
resolver rather than by editing user settings.
