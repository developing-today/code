# Embedded playable posts — HTML5 mini-games, async challenges, forkable succession saves

**Date:** 2026-10-30 · **Issue:** #2053 (LIST 15b) · **Epic:** #2056 · **Feeds synthesis:**
#2054 (boards v1, with boards investigation #2052)
**Scope:** the game-post object (package + mode + state lineage), async turn passing as a
signed journal, save points and the handoff/fork chain for succession play, attestation
plug-in (#2037), three worked examples. Out of scope: board objects themselves (#2052),
chunk retrieval internals (#2050), live-session hosting/discovery (flagged for #2054),
and the boards v1 synthesis itself (#2054 consumes this).
**Method:** this doc records the game-post model first produced by queue task 3358
(2026-10-29, on the issue) as the wave-pattern investigation document, re-verified
against `origin/main` (`b0058ec7`): the tic-tac-toe `.wasm` and roc examples exist, the
world capability/resource docs exist under `pkgs/id/doc/`, and the referenced issues
stand as scoped (#2026 closed, #2037/#2050/#2054 open, PoC #2266 open). This is a model
sketch, not an implementation and not a PoC.

---

## TL;DR verdict

A game post is a **world package + a play mode + a state lineage**, and ID already has
all three pieces. ID worlds are sandboxed, journaled programs (Roc/Wasm) in shared
worlds (`pkgs/id/examples/` — tic-tac-toe ships a `.wasm`; `roc-world/`, `territory/`,
`grid/` show the pattern), moves inside them are signed events on per-author chains, and
the docs already define world capability ceilings and resource profiles. So a playable
post is **not a new runtime**: it is a board post whose payload binds (a) a game
_package_, (b) a _play mode_, and (c) a _state lineage_ in the graph. The
investigation's real work is the mode/state design, below.

---

## 1. The game-post object

`{package_ref, config, mode, roles_policy, genesis_state | save_point_ref, attestation,
board_ref}`, signed by the poster.

- `package_ref`: app/world package ID + version hash, per the #2026 app-packaging model
  (closed). **The package is referenced, never inlined.** Posts stay small; the package
  is content-addressed, cached, and shared across every post of that game. A package
  update is a new version hash — old posts keep running the version they name (a chess
  post from last year must not change rules mid-game).
- `config`: board/map/options the package defines.
- `mode`: `live-session` | `async-turns` | `succession`.
- `roles_policy`: who may play, spectate, fork — IDs/groups, #2030-style writer sets.
- `genesis_state` or `save_point_ref`: where this instance starts.
- `attestation`: `none` | `concession` | `witnessed` (see §4).
- `board_ref`: the post/thread it lives in (`embed_ref` hook from #2052's boards model).

**Playable ≠ trusted.** The package runs in the world sandbox under the existing
capability-ceiling/resource-profile model: no ambient filesystem/network, capabilities
only as the viewer's client grants. The poster's signature gives provenance, not safety;
a board that embeds packages is making the sandbox earn its keep, and package-level
abuse (resource exhaustion inside ceilings) is a #2032 budget question. This must be
stated in #2054, not discovered in production.

---

## 2. State and turns (async-turns mode — the board-native mode)

- A game instance's state is a **journal lineage**: genesis state → turn objects, each a
  signed event `{instance, turn_no, move (package-defined payload), prev_frontier}` by
  the player whose turn it is. Current state = the package's deterministic fold over the
  journal — any holder of the journal can compute it, replay it, or audit it (the same
  operation as #2038's replay).
- **Turn enforcement is structural:** a turn object is valid only if it names the
  current frontier and is signed by the player the package's rules say moves next;
  competing turn objects at one frontier are a fork — visible, attributable, and only
  one line can be _the_ line under the instance's counting policy (the board/thread view
  decides which line it displays, exactly the #2030 counting-view pattern; the other
  line still exists as a fork anyone can follow).
- **Asynchrony is free:** turns are ordinary signed events — they sync whenever the
  players sync (hours, days; the chess-by-mail case). No session, no server, no presence
  requirement. A "your move" notification is a view over journals a user participates
  in, not a protocol feature.
- **Spectating** is subscribing to the journal: read-only, and the default for board
  readers — a game post renders as a live board position for everyone, playable only by
  its players.

---

## 3. Save points and forking (succession mode)

- A **save-point object**: `{instance, frontier, state_hash, manifest (package version,
player roster, in-game clock/era), note}` signed by the current holder. It is a
  _named, citable state_ — the unit succession games pass around.
- **Handoff:** `{save_point, from, to, terms (e.g. "play up to 10 in-game years, then
pass on")}` signed by the giver; the receiver's **acceptance object** starts their
  leg. The succession _chain_ is the linked sequence of handoffs + acceptances — who
  held the fortress, when, and under what terms is the game's public history, rendered
  by the thread.
- **Forking is a right, not an exception:** anyone entitled to read a save point may
  publish a fork object `{save_point, new_instance}` and play their own continuation.
  Forks carry their origin by construction (the save point names its lineage), so
  attribution is automatic and "the original line" is a _social_ fact — the thread's
  counting view follows the accepted handoff chain; forks are alternate timelines listed
  alongside, not rivals the protocol must suppress. This is git's social model applied
  to forts, and it is the only version of "forkable and built in" that doesn't need a
  central arbiter.

---

## 4. How attestation plugs in (#2037)

Attestation is a **per-instance option declared in the game post**, not a layer every
game pays for:

- `none` — casual posts; the journal speaks for itself and nobody certifies a winner.
- `concession` — at game end, the #2037 flow verbatim: the game instance's genesis
  doubles as (or references) the match agreement, result claims are signed by both
  players, `agreed`/`final` computed from objects.
- `witnessed` — ranked/competitive posts: agreement names witness + logic hash (=
  package version, elegantly: the game _is_ the logic), witness replays the journal to
  decide disputes.
- Because turns are already a signed journal, attestation adds only the agreement +
  claims — measured in objects, the plug-in is small; measured in trust, it's the
  difference between "a game happened" and "a result counts" in a registry's view
  (#2037/#2030).

---

## 5. The three examples

1. **Multiplayer eat-the-dots (agar-style).** The odd one out: it is **real-time**, so
   its mode is `live-session`, not async turns. The post embeds the package + a session
   rendezvous (a hosted or P2P world session per the messaging/relay work); play state
   is an ephemeral stream with periodic signed state snapshots anchoring the session
   into the graph. End of session: a final standings snapshot, attestable in
   `concession`/`witnessed` mode if the post declared it. Honest constraints for #2054:
   session hosting (who runs a persistent world — poster, board host, rotating player
   host) and tick latency are _live-systems_ questions this post model deliberately does
   not solve; the post model's job is that joining, spectating, and the result all have
   graph representations. A board full of live-session posts also needs session
   discovery ("3 games running now") — a view over session announcements, flagged as
   #2054 scope.
2. **D&D-style role-playing threads.** Mode `async-turns` with a twist: the roster has
   **roles with asymmetric authority** — the DM's state objects (scene changes, NPC
   moves, rulings) are authoritative for the campaign instance; player turns are actions
   _within_ their characters. Campaign state (character sheets, inventory, world facts)
   lives as graph objects the package fold references, so a character is portable across
   threads. Dice need care: a roll must be a signed roll object, and for anything
   competitive the honest options are DM-signed rolls (trust the DM — the social
   default, and fine) or commit-reveal between parties (for player-vs-player contests);
   "trust me, I rolled a 20" as unsigned text is the failure mode the objects exist to
   prevent. Mostly, though, this example proves the model bends: freeform narrative
   posts and structured state objects interleave in one thread, and the fold only
   consumes the structured ones.
3. **Dwarf Fortress succession.** Mode `succession`, the handoff chain above, designed
   against the actual ritual: the thread opens with a genesis save point; each ruler's
   leg is turns/journal under their sole authorship (single-player legs are the
   degenerate async case — one writer); their final save point + a handoff object
   naming the next ruler closes the leg. Terms ("N in-game years") live in the handoff
   and are _socially_ enforced — the journal's in-game clock makes overruns visible, and
   the thread's counting view can mark a leg that overran; the protocol provides
   evidence, the community provides judgement, as with everything else in this program.
   Forking shines here: a famous fort's year-10 save point can spawn a dozen alternate
   timelines, each a first-class instance with lineage, while the canonical chain
   continues in the thread. Save _files_ themselves are blobs (content-addressed,
   chunked retrieval #2050) — the save-point object references the blob hash; the graph
   never carries the fortress inline.

---

## 6. Open for the synthesis (#2054)

Live-session hosting/discovery (example 1's real cost); whether `roles_policy` needs a
general game-roles vocabulary or stays package-defined (recommendation:
package-defined, with the DM case as the forcing example); blob-size economics of save
files; how boards rank/surface game posts vs text posts.

## 7. Follow-on routing

- PoC follow-on **#2266** (game-post with async turn passing on tic-tac-toe:
  challenge/acceptance, signed turns, concession result, save-point fork) was filed by
  task 3358 and is open — this write files nothing new. PoC only, per program rules.
- Findings feed **#2054** by link; #2054 was gated on #2052 as well, and #2052's doc
  has now landed (PR #2810, merged) — the gate this investigation was waiting on is
  open from this issue's side.
- **Not a PoC:** no code was built; the model above is the deliverable.

## Verification note

Checked at `origin/main b0058ec7`: issue #2053 open with no owning or duplicate PR;
`pkgs/id/examples/tic-tac-toe/` ships `main.roc` + `tic-tac-toe.wasm` alongside
`roc-world/`, `territory/`, `grid/`; world capability/resource docs present under
`pkgs/id/doc/` (2026-10-16 entries and later); #2026 closed, #2032 closed, #2037/#2050/
#2054/#2266 open as scoped. Prior sketch: task 3358's comment of 2026-10-29, recorded
here as the wave-pattern doc with no change to the model.
