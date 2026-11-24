# Issues survey + build verification — FLEET-SIDELINE, 2026-11-23

## Issue/PR record used (via fleet gh api, sandbox seat)

- #13900 (open): PM book programme. Body: fifth book in pkgs/pm-stack/docs/;
  completion test = every worker + every Turso DB covered in PM book or one
  of the four existing books; "Bridge: light coverage here; its durable
  home is the fleet book + a fleet subcommand (follow-up issue filed by
  this programme)."
- #13404 (open): commander tracking. Body = original design (Worker,
  commander.security.cab Access app owner-Google + service-token policy,
  commander-link service-token-only, tunnel -> localhost:8931,
  supergateway unit, secret names incl.
  cloudflare.access.desktop-commander-client-id/-secret — NOTE: body
  assigns that pair to the bridge door, but the landed worker README
  assigns the -link- pair to the bridge door and the non-link pair to the
  retired public door; chapter follows the landed source).
  Comments: 2026-11-23T16:47Z build complete, endpoint live, publish run
  45218020597 + promote run 45220046876 green, deployment a2bbae9b serves
  version 37e2f6a3; 2026-11-23T17:16Z OAuth front door built/deployed/
  verified, PR #13488 merged 4421467c3776.
- PR #13488 (merged 2026-11-23T17:08:25Z, merge commit
  4421467c37767acf69f3b288744681155c1a563d): OAuth 2.1 front door —
  discovery, RFC 7591 registration, /authorize behind Access (app
  re-scoped), PKCE S256 code 10 min single-use, access 1 h, refresh 30 d
  rotating, KV by SHA-256, MACHINE_BEARER secret, 401 resource_metadata.
- #13427 (merged): original endpoint build (Worker, bridge package,
  publish/promote Actions).
- Commander issue search (title): open #13404, #13654, #13699; closed
  #13472 #13473 #13684 #13441 #13478 #13811 #13453 #13488 #13447 #13443
  #13429 #13479 #13716 (+ older mcpx ones #863 #3384 #897).
- #13654 (open): propose fleet commander subcommand; body notes route
  order (Desktop Commander first — only route whose results do not trip
  the platform execution-risk check). Implemented via #13717 (fleet
  commander health/exec, T1) — D2.
- #13699 (open): restore '' delimiters on desktop-commander-bridge-start
  (main-broken #13670); claims 83fa1a3 added the unit without them.
  Current main file HAS both delimiters (direct read); #13716 closed a
  sibling restore — D3.
- Bridge issue searches: "fleet bridge" in title -> #13213, #12168,
  #12167, #13197, #12199, #12170 (all closed). bridge+watch in title ->
  none relevant. "class/name" -> #13895 only. Conclusion: no existing
  issue for the notes subcommand family -> filed #13907.
- #13907 (filed by this lane via ~/tools/gh-post create, posted (user),
  read back via fleet gh api: open, title "fleet bridge:
  send/get/inbox/watch for class/name notes — replacing ad-hoc bridge
  access"): https://github.com/developing-today/code/issues/13907

## Transfer + patch mechanics

Drafts written in sandbox (~/workspace/fleet-cmdr-bridge-book/), moved to
the laptop as base64 in 6000-char argument chunks via laptop-ssh.sh,
sha256-verified both sides (commander.md 4f0e08f2bfa126f45ab1656df86ff29
2184a54726462965add832a7f0b9c2056; one chunk initially diverged and was
re-sent in halves until the full-file hash matched). Edits to existing
chapters applied on the laptop by a sha-verified python patch script
(exact-string replaces, fail-loud on miss; all 7 patches applied).

## Build verification actually run (in the worktree, laptop)

1. `mdbook build` (pkgs/fleet, dest /tmp/fleet-book-render): exit 0,
   WARNCOUNT=0 (grep -ci warn on the log). Rendered checks:
   concepts/commander.html exists, concepts/bridge.html contains the new
   section heading + muse/archer; placeholders render escaped
   (&lt;class&gt;, at:&lt;hash&gt;, client:&lt;id&gt;).
2. treefmt --fail-on-change on the 4 touched files: reformatted only
   commander.md (table padding, 17318 -> 25638 bytes); other 3 clean.
3. After formatting: mdbook build again — 0 warnings;
   `nix build .#fleet-docs --no-link --print-out-paths` green:
   /nix/store/69zsm1mffr27fzkc8xp30lq0469ypyhf-fleet-docs
   (first pre-format build: /nix/store/fny26bfkf2jj53ryz0i5l74yi83rq1xs-fleet-docs;
   the nix "Git tree is dirty" line is the standard uncommitted-flake
   notice, not an mdBook warning).
4. Hedge grep (reportedly / appears to / -shaped) over the 3 changed
   chapters: no hits.
5. Commit ec64fd8553e8cd639ab95ab0e44275bf442a5c4c on
   book/fleet-commander-bridge (parent 0ecb31eef33 = origin/main at
   worktree creation), pushed; PR #13915 opened via laptop gh (body
   backticks were shell-substituted by the transfer command — repaired
   in place with a sha-verified python script, all 3 fixes confirmed);
   merged via laptop gh --merge: merge commit
   189c055fee2a3ededbc4f14432953d821a6af410, mergedAt 2026-11-24T00:51:18Z.
