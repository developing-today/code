# Routing Policy

Adopted 2026-11-16 on #2169 (pending-decisions ledger item `routing-policy-2169`):
the fleet's **de-facto behavior is the policy**. This file supersedes the
issue's original draft, including its "switch model in place" section.
Pool definitions and per-class fallbacks live in `.opencode/opencode.jsonc`
and `.opencode/route-fallbacks.md`; this file is the session-facing rule.

## 1. Tiers and escalation

- **Workhorse is the default route** for routine dispatched work (the config
  default `model` is `opencode-route/workhorse`).
- Roles and agent definitions state **requirements only** (a strength floor
  and a speed flag); named-model pins in role/agent defaults are banned
  (stewardship Decision 008). The resolver picks the model live from pool
  state.
- **Specialist** tier is for design and deep-reasoning work.
- **Summit is not a general escalation tier.** Under the design-mill pattern,
  ONE summit session OWNS a large multi-part design item end-to-end and
  delegates everything it can spec to workhorse subsessions; Muse does the
  intake (starter issue, docs, queue item). Summit use is therefore
  structural and visible on the item it owns, not a per-task upgrade.

## 2. Model switching: there is none

- **A session runs the route head it was launched with. No in-session model
  switch exists, and none should be attempted.**
- Escalation happens by **launching a NEW session on the appropriate route**,
  with the model and effort chosen deliberately at launch (by plan/quota size
  and availability — never left at default).
- **Record the reason in the session record** — the launch prompt / task note
  states why the higher route was chosen (for example
  `escalated to specialist: policy-wording review`). A launch whose reason is
  not recorded is not a policy escalation.

## 3. Subsession pool selection

- A dispatching session does **not** pick pools per subsession: it dispatches
  on the `workhorse` route and lets the **route own pool selection and
  failover**.
- Dispatch is gated on a quota read **≤60 minutes old** for the pool. Pools
  burn toward 100% at their reset, soonest-expiring first. A failed or stale
  quota read never idles a pool: probe inference applies, and dispatch runs on
  last known state.
- **Plan quotas only.** A quota error is a wall, never a pay-as-you-go
  fallback. The sole named exception is the DeepSeek direct reserve, used when
  every plan is used up.

## 4. Pool awareness: managed routes first

- Managed pool routes are **preferred over individual model pins**.
- A pin stays allowed **with a stated reason** — a capability only that model
  has.

## Readiness and evidence

- Every `subagent` launch names the route (or the documented reason for a raw
  model) in both the launch `description`/prompt and its record.
- Acceptance demonstrations for this policy live on #2169: an escalation by
  fresh launch on a higher route with the reason logged, and a subsession
  spawned on the `workhorse` pool route with route-owned selection.
