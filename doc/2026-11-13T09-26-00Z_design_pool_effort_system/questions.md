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
