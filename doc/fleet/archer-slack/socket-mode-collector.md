# Event-driven Slack collector (Socket Mode) — design record for #2137

Status: collector code landed in-repo (`pkgs/fleet/archer-slack/collector/`);
deployment and the parallel run are the remaining slices below. Companion
records: `slack-agent-design.md` (the Archer app + puller design this
extends), the 2026-11-13 parallel-run fidelity report on #2137 (comment
6847788414), and the app manifest in this folder.

## Why

`archer-slack-review` wakes every 15 minutes, pulls the watermark delta
with `pkgs/fleet/archer-slack/puller/puller.py`, and hands the delta to an
agent run: ~100K input tokens per run × ~96 runs/day ≈ 9.6M tokens/day,
usually to find nothing. #2137 (Drew approved 2026-10-29) replaces the
poll-driven wake with an event-driven one: a deterministic collector
appends watched-channel events to the store as they happen, and the
review job wakes only when the store has new items. No LLM anywhere in
the collection path.

## What already exists (verified 2026-11-22)

- **The puller + review job** are live: `archer-slack-review`
  (interval@15m, Muse cron on the sandbox seat, row in
  `doc/fleet-schedule-registry.md`) runs the puller against the SQLite
  store (`collected_events`, `seen`, `watermarks` tables).
- **The collector PoC was built 2026-10-29** and ran live once: capture
  2026-10-29T07:27–07:46Z, **10/10 in-window events matched the puller's
  seen-set exactly, including 5/5 Aster replies**, dedupe safe. Capture
  then ceased because the sandbox host recycled and the service did not
  survive; the 3-day parallel run never happened. Cutover recommendation
  of record: **DO NOT CUT OVER** until a fresh parallel run passes.
  Until this change the code lived only in a sandbox workspace; it is
  now in the repo next to the puller, with its tests.
- **The Slack app already supports this**: the manifest in this folder
  has `socket_mode_enabled: true` and bot events for messages
  (channels/groups/im/mpim) and mentions. The app-level token the
  collector needs is fetched by name (`slack.archer-app-token`) from
  Bitwarden Secrets Manager at service start — never written to disk,
  logged, or printed.

## The collector as built

`collector.py` holds a Socket Mode WebSocket (`apps.connections.open`
with the app-level token; `websockets` is the only non-stdlib
dependency, pinned in `requirements.txt` to the PoC-proven version). It
acks every `events_api` envelope, maps message and reaction events to
rows in the puller's own store (`collected_events`), and dedupes on the
events' natural identity — `(channel, ts, event_type, user_id,
reaction)` — so collector rows and puller rows are directly comparable
for the fidelity check, and replays are harmless. Watched channels are
a parameter; the PoC channel is #alignment. Fatal auth errors exit
loudly (78) instead of retrying forever; transient drops reconnect with
bounded backoff. The review job, the store schema, and the watermark
contract are untouched by the collector: during the parallel run both
writers append to one store, which is what makes the comparison a read,
not a reconstruction.

## Decisions

1. **Hosting: the laptop, via the code flake.** The issue's placement
   rule is "developing-today/code flake if on the laptop; sandbox
   systemd if on the sandbox". The sandbox branch is impossible, not
   merely inconvenient: this sandbox has **no systemd user bus at all**
   (`systemctl --user` → "Failed to connect to bus", verified
   2026-11-13, 2026-11-16, and again 2026-11-22), and the 2026-10-29
   live run demonstrated the second failure mode — the sandbox host
   recycled ~54 minutes in and neither the unit nor its journal
   persisted. The collector is a supervised-daemon-shaped workload;
   it belongs on the host whose service state persists. Deployment is
   a flake module + systemd user unit on the laptop, with the
   schedule-registry row added in the same change that creates the
   unit (a schedule on a surface but not in the registry is a defect).
2. **Wake path: store-first, puller as fallback.** The review job's
   cheap pre-check reads the store: if `collected_events` has rows
   newer than the review watermark **and** the collector's heartbeat
   is fresh, the run reviews the delta and skips the pull. If the
   collector is stale or absent, the job falls back to today's pull —
   the event path can fail safe to the status quo, never to silence.
   (Wiring this gate into `archer-slack-review` is slice S3; the job's
   body lives on the owning seat, so S3 is a seat change recorded
   here, not a repo edit.)
3. **Reactions need a manifest amendment.** The collector stores
   `reaction_added` events, but the app manifest does not subscribe
   to them (bot events list messages/mentions only) and the bot lacks
   `reactions:read`. Until the workspace owner adds the
   `reaction_added` subscription + `reactions:read` scope and
   reinstalls the app, reaction capture is code-ready but event-starved
   — the same reason reaction fidelity could not be assessed in the
   2026-11-13 report. This is a named owner action, not a code gap.
4. **Cutover is earned by a parallel run, not by this design.** The
   acceptance protocol is the 2026-11-13 report's own remainder: the
   collector runs on the laptop for **3 days in parallel** with the
   puller; a fidelity report compares the collector's captured set
   against the puller's seen-set for the window (messages and thread
   replies, Aster replies called out explicitly), plus collector
   uptime/restart counts from the laptop journal. Zero missed Aster
   replies and no unexplained capture gap → cutover slice proceeds;
   anything else → the puller stays primary and the gap is diagnosed
   first.

## Slices

- **S1 (this change):** collector code, tests, and launcher in the
  repo beside the puller; this design record.
- **S2:** flake module + systemd user unit deploying the collector on
  the laptop against the same store path the review job reads;
  schedule-registry row in the same change.
- **S3:** wake-gate in `archer-slack-review` (decision 2), on the
  owning seat.
- **S4:** the 3-day parallel run + fidelity report on #2137.
- **S5:** cutover — the review job stops pulling when the collector is
  healthy; the puller remains in the repo as the fallback tool. Only
  after S4 passes.

## What this does not change

The review job's cadence, prompt, and authority; the puller's
watermark contract; the store schema. Quiet-day agent cost reaches ~0
only when S3+S5 land — S1 alone changes no live behaviour, by design.

## S2 deployment (2026-11-22)

The deployment form, all in this repo:

- **Package:** `pkgs/fleet/archer-slack/collector/package.nix`
  (interpreter + websockets + `launcher.sh`, the service entry point:
  token by name via `fleet secrets get`, store directory ensured,
  then exec). `nix build .#archer-slack-collector` is the package
  check (lib/default.nix `make-archer-slack`).
- **Module:** `nixos/services/archer-slack/default.nix`
  (`services.archer-slack-collector`), a home-manager user service
  like the pm-stack units; enabled for the laptop host (`amd`) in
  `nixos/hosts/default.nix`. The unit sets `ARCHER_SLACK_DB` to the
  store path the review job reads
  (`~/workspace/archer-slack/puller/state/archer-slack.db`, the
  laptop-local copy the parallel run writes) and
  `ARCHER_WATCH_CHANNELS` to the PoC channel; Restart=on-failure.
- **Registry:** the unit's row is in `doc/fleet-schedule-registry.md`
  Surface 2b, added in the same change as the unit.

First activation was a narrow unit-level install on the laptop (the
unit file home-manager will generate, installed directly) because a
full `./rebuild` had other work pending; the landing note on #2137
records the exact start time of the 3-day parallel run and the
rollback step. A later rebuild from a main that includes this change
must preserve: the same unit (home-manager takes over the path,
renaming any hand file to `*.backup`), the store file (outside
home-manager's management; it persists), and the verified-live state
— re-check `systemctl --user status archer-slack-collector` and its
journal after that rebuild, since a unit that comes back wrong
invalidates the parallel-run window it interrupts.

The reaction gap from decision 3 stands: no `reaction_added`
subscription, no `reactions:read` scope — reaction capture remains
event-starved until the workspace owner amends the app manifest and
reinstalls. S2 deploys with the subscriptions that exist (messages
and mentions), which is what the parallel run measures.

## Decision record completion (2026-11-24)

### Goals and non-goals

**Goals:** reduce idle review-agent wakes by collecting Slack events
continuously; preserve the puller's SQLite event identity and dedupe
contract; deliver the #alignment PoC with measurable fidelity before any
cutover; keep collection deterministic and recoverable.

**Non-goals:** change review policy, prompt, cadence, or authority; replace
the puller before the parallel-run acceptance gate; expose a public webhook;
add an LLM to collection; or enable reaction capture before the workspace
app's scopes and subscriptions are amended.

### Alternatives considered

1. **Keep the 15-minute watermark poll (do nothing):** lowest operational
   risk and remains the fallback, but retains roughly 9.6M mostly-idle input
   tokens/day.
2. **Slack Events API through a Cloudflare Worker:** moves the persistent
   process off the laptop, but needs a public endpoint, signature validation,
   deployment/secrets management, and another durable delivery path. Not
   selected for this local-store PoC.
3. **Socket Mode collector on the laptop (selected):** outbound WebSocket
   avoids a public endpoint and appends directly to the review store. It
   depends on laptop availability and service supervision; stale-heartbeat
   fallback to polling bounds that risk.

### Interfaces, data, and security contract

The collector receives Socket Mode envelopes, acknowledges each
`events_api` envelope, filters to configured watched channel IDs, and writes
the existing `collected_events` row shape. Natural identity is
`(channel, ts, event_type, user_id, reaction)`; retries are idempotent. The
review-side interface is the existing SQLite store plus a freshness
heartbeat; the collector does not invoke the reviewer or mutate review
decisions. `ARCHER_SLACK_DB` selects the shared store and
`ARCHER_WATCH_CHANNELS` selects the allow-list.

The app-level token is fetched by secret name at service start, not placed
in the unit environment, command line, database, or logs. Use the least
Slack scopes/events needed for the PoC; do not subscribe to unrelated
channels. SQLite remains local to the laptop account with its existing file
permissions. Reconnect/backoff handles transient transport loss; fatal
authentication/configuration errors stop visibly rather than silently
pretending collection is healthy. The fresh-heartbeat condition is
mandatory before the review job may skip its poll.

### Dependency-ordered slices and acceptance

| Slice | Depends on | Acceptance |
| --- | --- | --- |
| S1: collector code, tests, launcher, and this record | — | Unit tests cover message/reaction mapping, channel filtering, envelope acknowledgement, dedupe, reconnect, and fatal auth; code writes the puller's schema. |
| S2: laptop flake package/module and schedule registry | S1 | User service starts under systemd, reads the named token, writes the reviewer's configured DB, restarts on failure, and appears in the schedule registry. |
| S3: review wake gate | S2 | Fresh collector heartbeat plus new rows causes delta review without a pull; stale/missing heartbeat always invokes the existing puller. |
| S4: three-day parallel run | S2 (S3 may remain disabled) | A dated report compares the collector set with puller `seen` for the same window; zero missed Aster replies, all message/thread-reply differences explained, and no unexplained capture gap. Include uptime/restarts. |
| S5: guarded cutover | S3 and passing S4 | Quiet-day review-agent wakes attributable to polling reach zero; puller fallback remains callable and verified. |

The S1/S2 implementation and PoC evidence are documented above and in the
linked #2137 report. S3–S5 remain gated work, not implied by this design
record.

### Rollout and rollback

Roll out S2 alongside the existing poller; do not disable or reschedule the
poller during the parallel run. Verify service health and database path,
then collect for a complete three-day window. Enable S3 only with the
fresh-heartbeat guard and keep the puller as fallback. S5 is a separate
change after S4 passes. To roll back, disable the wake gate first (restoring
poll-on-schedule), then stop/disable the collector service. Preserve the
SQLite file and collected rows for diagnosis; do not delete or rewrite the
store. A restart or unexplained capture gap invalidates the affected run
window and restarts the three-day acceptance window after recovery.

### Verification plan

- Run `python3 -m unittest discover -s pkgs/fleet/archer-slack/collector -v`
  and the puller tests; verify package evaluation/build on the Fleet runner
  before changing deployment.
- On the laptop, inspect `systemctl --user status
  archer-slack-collector` and its journal; confirm the configured DB path,
  a fresh heartbeat, and no token disclosure.
- During S4, compare event identities against the puller's `seen` set over
  identical timestamps/channels, separately account for Aster replies, and
  record service uptime/restarts. Acceptance is the table's S4 criterion,
  not merely a successful process start.
- After any rebuild/restart, re-check the unit and journal; interrupted or
  unhealthy intervals cannot count toward the three-day window.

### Open questions and deciders

- **When to grant `reaction_added` and `reactions:read`:** workspace/app
  owner decides and performs the app amendment/reinstall. Until then,
  reactions are explicitly outside measured acceptance.
- **Whether to generalize beyond #alignment:** Drew decides after the
  #alignment S4 report; default is no expansion until fidelity passes.
- **Whether to proceed with S5:** the issue owner decides after reviewing
  the S4 report; default is to retain the poller as primary if any Aster
  reply is missed or any capture gap is unexplained.

The tracking issue is [#2137](https://github.com/developing-today/code/issues/2137);
this dated record is its design reference. No deployment or cutover is
authorized by documenting these gates.
