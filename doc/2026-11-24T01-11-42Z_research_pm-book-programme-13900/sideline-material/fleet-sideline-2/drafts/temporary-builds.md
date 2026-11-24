# Temporary builds and custom connections

<!-- id: temporary-builds since: pre-release -->

A temporary build is a machine the fleet does not own, borrowed for
minutes: a GitHub Actions runner, started for one session, reached
through a connection built for that session alone, and gone when the
lease ends. The machinery is `fleet actions exec` (#13774): live,
short-lived exec sessions inside Actions runners, in the
#13717 route-family shape — a timeout required by construction, and
the serving form named in the output. The verb-by-verb reference is
[fleet actions](../reference/actions.md); this chapter is the design:
the handoff, the two clocks, the tiered extension policy, and the
door the runner dials in through.

Drew's order (2026-11-23 15:17 CST) was for an alternate exec mode
that may become the primary: agents connect into Actions runners and
exec commands themselves, temporary and short-lived like the
commander and route subcommand family, with the central coordinator
(#13765) establishing the connection handoff and the fleet command
background-watching the run after handoff.

## Why the runner dials out

The design was chosen under one hard filter: most off-the-shelf
runner-access methods print SSH or connection information into the
run log, and in a public repo the log is world-readable. Any method
whose connection material can appear in logs is disqualified, however
convenient. The survey in #13774 records the verdicts: tmate and
upterm rejected on exactly that leak; the Tailscale GitHub Action
viable but not picked (it couples runner access to tailnet enrolment
for every caller, and the transport design #13752 is moving away from
Tailscale-only reach); cloudflared with Access kept as door
technology, not as the session protocol; an iroh endpoint on the
runner the right long-term transport, but not the v1 session protocol.

The chosen shape inverts the connection: **the runner-side agent
dials out to the coordinator.** The workflow starts a small agent —
a mode of the fleet binary, `fleet actions agent`, invoked by the
`fleet-exec-session` composite action in dezren39/fleet. The agent
proves it is inside the run with the job's GitHub OIDC token, fetched
from the Actions runtime and never logged; it registers with the
queue broker and holds an outbound long-poll. Commands flow
requester → broker → agent; IO flows back the same way. No inbound
path is needed, nothing connection-shaped ever exists outside the two
endpoints, and the session handle is a public identifier — safe in
logs, safe on the run page. Workflow inputs carry only the handle
and non-secret parameters, never a token: inputs are visible on the
run page.

Three pieces make one flow:

1. **The action** — dezren39/fleet `.github/actions/fleet-exec-session`
   (composite): sets up the pinned fleet binary, runs
   `fleet actions agent` for the session, and ships the egress helper
   `fleet-exec-put` on the session PATH.
2. **The workflow** — dezren39/fleet `.github/workflows/fleet-exec.yml`
   (`workflow_dispatch`; inputs are the handle, an optional target
   repo/ref checkout, the runner, the lease and max seconds, and the
   hide-logs flag; `permissions: id-token: write`; the job timeout is
   the max lifespan plus grace).
3. **The subcommand** — the `actions` domain in the fleet binary:
   `fleet actions exec start / run / attach / watch / renew / extend /
   decide / close`, plus `fleet actions put <file>` for egress.

## The handoff: workflow id, run attempt, handle

The handoff is the triple a requester keeps: the **session handle**,
the portable **`OWNER/REPO#RUN_ID`** form, and the **run attempt**.
`fleet actions exec start` asks the coordinator to claim a slot for
the session's host surface and create the session row — composing
with #13765's design, never forking it: when the coordinator's
`/placement-claim` answers granted, the session proceeds on the
coordinator path; when it does not answer, the session proceeds on
the singular per-seat path and the output names which happened via
`placement_path`. The subcommand then dispatches `fleet-exec.yml`
with the handle, and binds the resulting run id and attempt to the
session row (`/actions-exec-bind`). Sessions are recorded in the
local store under the state dir, so the portable form resolves later,
from any seat.

On the runner side, registration is the mirror image of the bind:
the broker verifies the agent's OIDC JWT against GitHub's JWKS and
checks its `repository`, `job_workflow_ref`, `run_id` and
`run_attempt` claims against the session row created at dispatch.
Requester-side auth is the existing broker seat auth. The broker
surface is twelve verbs in
`crates/fleet-queue/src/turso_queue/actions_exec.rs`, routed beside
placement's: seat-side `/actions-exec-open`, `-bind`, `-command`,
`-stream`, `-renew`, `-extend-request`, `-extend-decide` and `-close`
under the broker token; agent-side `/actions-exec-register`, `-poll`,
`-io` and `-result` under OIDC. Session state lives in the
`actions_exec_sessions`, `actions_exec_commands` and
`actions_exec_io` tables, and every transition is a conditional
UPDATE, so two actors racing a session resolve to exactly one winner.

Disconnect tolerance follows from the broker being the transport: a
requester disconnect is a non-event (the broker holds the commands,
the IO and the lease), agent network blips retry with backoff against
the long-poll, and reconnecting is re-attaching with the same handle.
Files leave through `fleet actions put`: into the session outbox,
uploaded as the run's `fleet-exec-out` artifact, and — when the
workflow populated the `FLEET_EXEC_R2_*` environment from masked repo
secrets — PUT to R2 under `fleet-exec/<handle>/` with a SigV4
signature, skipping quietly when the credentials are absent.

Log visibility is a choice, not a leak: by default the agent echoes
command IO to its own stdout, so it lands in the public Actions run
log; `start --hide-logs` turns the echo off. Either way the broker's
IO chunk store is the cache of record. The OIDC token and the R2
credentials are never written to any log, and reading the produced
public logs end to end for connection material is part of the mode's
acceptance bar.

## The door: exec.security.cab is a tunnel ingress, not a Worker

The agent's verbs need a public route to the broker, which lives on
the laptop. That route is `exec.security.cab` — and it is **not** a
Cloudflare Worker. The WORKER-EVIDENCE survey for programme #13900
settled this first-hand: of the security.cab doors, only
`commander.security.cab` and `mcpx.security.cab` are Workers, each
with a wrangler config in the repo. The exec door has no wrangler
config, no OAuth front door and no Access app, because there is no
Worker to configure: the sandbox-link tunnel is remotely managed, so
its ingress lives in Cloudflare, not in the repo.

What the door is, as deployed (recorded on #13774, Slice 5,
2026-11-23T22:10:06Z): the sandbox-link tunnel (id beginning
`722f72ba-`), at config version 6, gained one ingress rule —
`exec.security.cab` with path
`^/actions-exec-(register|poll|io|result|close)$` forwarded straight
to the laptop queue broker at `100.99.113.103:4801` — inserted before
the catch-all with every existing rule preserved, plus a proxied DNS
CNAME for `exec.security.cab` in the security.cab zone. There is no
Access app in front **by design**: the runner job's GitHub OIDC token
is the authentication, verified at the broker. The path scoping is
enforced at the tunnel and was verified from outside: a tokenless
`POST /actions-exec-poll` returned the broker's own
`401 {"error":"unauthorized"}`, while `POST /claim` and `GET /` on
the same host returned 404. In code, the door is a broker URL and
nothing more: `pkgs/fleet/crates/fleet/src/actions.rs` carries
`const DEFAULT_BROKER_URL: &str = "https://exec.security.cab";`.

## Two clocks: the lease and the max lifespan

Every session carries two clocks, both enforced by the broker and by
the runner-side agent, which exits at expiry even if partitioned from
the broker, after a final IO and egress flush:

- The **lease** — set by `start --timeout`, required, with no
  default. Renewable through the coordinator (`exec renew`), clamped
  at the max.
- The **max lifespan** — `--max-lifespan`, default 30 minutes,
  ceiling 2 hours, the fleet leg cap. Renewals move `lease_until` up
  to the max and no further; past it, only an extension helps.

## The tiered extension policy

Extensions past the max are governed by the tiered policy Drew set on
2026-11-23 (~17:06 CST), recorded on #13774 before implementation and
landed by PR #13854 (merged `f56dc7f713ef`, published as
`fleet-static-f56dc7f`, ACTIONS-EXEC-3 complete 2026-11-24T00:39:36Z).
Evaluation happens in `/actions-exec-extend-request` from the
session's extension count, its cumulative approved extension seconds,
and the raw ask. The tunables are named constants in
`turso_queue::actions_exec`:

| Tunable | Default | Meaning |
| --- | --- | --- |
| `EXT_AUTO_SIZE_CAP_S` | 1800 (30 min) | The largest single ask the auto tiers may grant; a raw ask above it is tier 3 even as a first extension |
| `EXT_TIER1_MAX_COUNT` | 1 | Extension #1 of a session is tier 1 |
| `EXT_TIER2_MAX_COUNT` | 2 | Extension #2 is tier 2, subject to the caps; #3 and beyond are tier 3 |
| `EXT_AUTO_TOTAL_CAP_S` | 3600 (60 min) | Cumulative approved extension seconds the auto tiers may grant over a session's life |
| `EXT_DENY_WINDOW_S` | 600 (10 min) | Tier 2's deny window: silence at window end approves |
| `EXTENSION_GRACE_S` | 300 | The decision grace past the lease while an extension pends |

- **Tier 1 — auto + notify.** The first extension, within the caps,
  is applied inside the request itself and recorded
  `ext_state = 'auto-approved'`. One notification-only push goes out:
  a plain PM alert with no needs-drew choice block.
- **Tier 2 — notify + deny window.** The second extension pends for
  the 10-minute deny window. The session's hard deadline covers the
  window — `max(lease, decide_after) + EXTENSION_GRACE_S` — so the
  agent is not killed mid-window. A deny through
  `/actions-exec-extend-decide` ends the extension at once; an approve
  applies it immediately; **silence approves at window end**, settled
  lazily by the next verb that touches the session (in practice the
  agent's poll), recorded `ext_state = 'silence-approved'`. Exactly
  one push per request: the deny-window warning is the push.
- **Tier 3 — manual only.** The original flow: the request pends with
  a 300-second grace past the lease, the broker raises a needs-drew
  alert (source `fleet-actions-exec`), and Drew's approve or deny
  returns through `exec decide`, adding up to +30 minutes to the max
  and the lease on approval.

The count and total increment on **every** approval — auto, silence
or manual — and never on a deny, so the ladder measures what was
actually granted. Every tier's push carries the detail Drew required:
the session handle and its recorded context link, the requesting
seat, the host repo, the workflow, run id and run attempt with a
click-through run URL, the agent's stated reason, the lease state,
the ask, the cumulative extensions before and after, the tier, and
what deny or timeout does at that tier. Alert keys carry the
extension ordinal (`actions-exec-ext:<handle>:<n>`) so successive
extensions never dedupe-suppress each other's pushes. The named gap,
unchanged from v1 and stated plainly in the design: the phone tap
does not write back to the broker yet — tier-2 denies and tier-3
answers land via a seat running `exec decide` on Drew's recorded
answer, and the 10-minute window is sized with that relay in mind.

## State of the build, 2026-11-23

Landed and deployed: the action and workflow in dezren39/fleet
(Slice 2, PR #112 there); the broker verbs, the typed client and the
subcommand (PR #13800, merged `5e4107076821`); the public door and the
R2 provisioning (Slice 5); publication as `fleet-static-5e41070` with
its bootstrap pin (PR #13817); and the tiered extension policy
(PR #13854). The lane's in-vivo proof battery halted at the Action's
agent-install step when reading the failed run's log tripped a
platform execution-risk check; that stop is recorded on #13774
(2026-11-23T22:57:46Z) with the exact slice state, and the end-to-end
live proof — exec on the runner, file out to both the artifact and
R2, disconnect/reconnect, renewal, and a short-lease session dying at
its cap — remains the open item this chapter does not claim.

## Sources

Issue #13774 (design of record, method survey, slice records, the
tiered-extension amendment at comment 7116645799, the ACTIONS-EXEC-3
completion record); `pkgs/fleet/crates/fleet/src/actions.rs`;
`pkgs/fleet/crates/fleet-queue/src/turso_queue/actions_exec.rs`;
`pkgs/fleet/crates/fleet-queue/src/client/actions_exec.rs`;
[the fleet actions reference](../reference/actions.md); WORKER-EVIDENCE
survey, programme #13900 (the exec door verdict).
