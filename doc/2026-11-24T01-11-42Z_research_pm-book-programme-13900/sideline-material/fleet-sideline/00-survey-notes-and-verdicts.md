# FLEET-SIDELINE raw survey notes — Commander worker + Bridge audit

Lane: FLEET-SIDELINE, programme developing-today/code#13900 (coordinator PM-BOOK).
Surveyed 2026-11-23 (CST) via the laptop clone `~/git/code` (primary at main
0ecb31eef33 after fetch; work done in worktree `~/wt-fleet-cmdr-bridge-book`,
branch `book/fleet-commander-bridge`), `fleet gh api` reads, and live
`fleet bridge` / `fleet commander` help dumps on the laptop.
Deliverable: PR #13915, merged 189c055fee2a3ededbc4f14432953d821a6af410.
Follow-up issue filed: #13907.

## Where the fleet book actually lives

The brief said `pkgs/fleet/docs`; the book is `pkgs/fleet/book/` (mdBook,
`pkgs/fleet/book.toml`, `src = "book"`, `create-missing = false`), built by
`pkgs/fleet/docs.nix` as flake output `.#fleet-docs`. `pkgs/fleet/docs/`
holds only two standalone pages (verbs.md, fleet-gh-credentials.md).
Deployed under /code/fleet/ in the combined mcpx-docs.yml Pages deploy.

## (A) Commander — verdicts

- No commander-door chapter existed. Only prior coverage:
  `book/reference/laptop-route-forms.md` documents the CLIENT (`fleet
  commander`, tier T1) — kept, cross-linked, not duplicated.
  `book/concepts/proxy.md` has zero commander mentions (grep-verified).
- Chain verified hop-by-hop against: pkgs/desktop-commander/worker/
  wrangler.toml, worker/README.md, worker/src/index.ts, worker/src/oauth.ts,
  worker/config.json, pkgs/desktop-commander/default.nix (version 0.2.52),
  home/common/default.nix (desktop-commander-bridge unit, supergateway on
  127.0.0.1:8931). Raw captures in this folder.
- `fleet commander --help` on the laptop: verbs `health`, `exec` — the
  subcommand EXISTS in the installed build.

## (B) Bridge — verdicts (audit, not rewrite)

Already existed and verified accurate for the mailbox system:
book/concepts/bridge.md, book/reference/bridge.md,
book/tutorials/bridge-messaging.md. Verified against live help dumps and
pkgs/message-bridge/worker/src/fleet.js + worker/README.md.
Stale/wrong items found (both in the tutorial only):
1. "single shared usage text" claim — per-verb --help landed (#13213,
   #13197). Live-verified: `fleet bridge inbox --help` prints per-verb text.
2. "group mailbox" claim — fleet.js `send` case expands group targets to
   one row per registered member at send time; `inbox` selects `agent = ?`
   only; the `groups` arg only refreshes registration via `touch()`.
   There is no group mailbox.
Missing entirely: the #13895 notes surface — added to concepts/bridge.md
as a marked in-flight section (design as recorded, dated 2026-11-23).
On main at survey time there is NO notes/signal code in
pkgs/message-bridge/worker/src/ (files: auth, events, fleet, index, mcp,
messages, oauth, page, relay, routes, wake — no notes module), and #13895
was filed 2026-11-24T00:18Z, after the clone's previous HEAD.

## (C) fleet bridge subcommand — verdicts

- `fleet bridge` EXISTS: send, agents, log, hello, inbox, post, ack, health
  (live dump in this folder). Closed history: #12167 (send/receive command
  channel), #13213/#13197 (per-verb help).
- The gap is only the #13895 notes surface. Drew's direction is recorded
  as a comment on #13895 (2026-11-23 ~18:29 CST): fleet subcommand family
  get/send/receive/watch replacing the standalone bridge helper.
- No existing issue proposed that family (title/body searches for
  "fleet bridge", bridge+watch, class/name returned only #13895 itself).
  Filed #13907 scoped to the gap.

## Numbered disagreements

- D1: #13404's body describes the superseded all-Access public-door design;
  PR #13488 (merged 4421467c3776) replaced it with the OAuth 2.1 front door
  (Access only on /authorize; public-door service token retired at the
  door). The chapter follows the source, and says so.
- D2: #13654 (fleet commander subcommand proposal) is open but implemented
  (fleet commander health/exec ship via #13717 route forms; live-verified).
- D3: #13699 (restore '' delimiters on desktop-commander-bridge-start,
  "main-broken #13670") is open but the defect is not present at current
  main: both delimiters are in home/common/default.nix (read directly);
  sibling fix #13716 is closed.
- D4: tutorials/bridge-messaging.md stale claims (shared usage text;
  group mailbox) vs shipped binary + fleet.js — fixed in PR #13915.
- D5 (minor, framing): the original lane brief located the fleet book at
  pkgs/fleet/docs and described bridge coverage as absent; both were
  corrected by the coordinator steer mid-lane (book at pkgs/fleet/book/;
  bridge chapters pre-existed). Recorded in the #13900 comment.
