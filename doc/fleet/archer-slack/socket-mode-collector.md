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
