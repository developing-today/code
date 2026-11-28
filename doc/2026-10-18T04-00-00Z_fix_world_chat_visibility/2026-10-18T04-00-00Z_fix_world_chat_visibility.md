# Fix: world chat is visible only to participants who may chat or read history

- Plan: [phase 11 foundations](../../.opencode/plans/phase-11-foundations.md) (the plan lives on another branch; the link resolves once it lands).
- Start commit: e1f99237 (origin/main, after #254).
- Branch: `fix-chat-visibility`.

## Cause

`WorldScopes::JOIN` was documented as "join the world and receive snapshots/events", and `WorldCore::snapshot` / `WorldCore::events_after` returned every retained event to every joined participant. A participant holding only `join` therefore read all chat, although it could not send any. The live stream was no better: a session forwarded each broadcast event to every connected participant without asking what that participant may read.

Issue #240 had already shown the same shape for the install seed, and #250 fixed it with `redact_install_seed`, applied in two places (`snapshot_unchecked` and `events_after`). Two inline copies of a redaction is how the live path was missed for chat.

## Decision (project owner)

Join-only participants must not see chat. A chat line is visible to a participant if their effective scopes include `chat` (already required to send) or a new explicit read scope. The install seed stays secret.

## The scope

`WorldScopes::HISTORY = 0b1_0000`, wire name `history`: "may read chat lines without being able to send them". `chat` also grants reading. `GUEST` is unchanged (it has `chat`). `ALL` now includes `HISTORY`.

| Scopes held                       | Reads chat | Sends chat | Notes                                               |
| --------------------------------- | ---------- | ---------- | --------------------------------------------------- |
| `join`                            | no         | no         | the old "spectator": snapshot and events minus chat |
| `join`, `history`                 | yes        | no         | an observer                                         |
| `join`, `chat`                    | yes        | yes        |                                                     |
| `join`, `chat`, `input` (`GUEST`) | yes        | yes        | unchanged                                           |

`chat` implies `history`. This is expressed once, as `WorldScopes::implied()` (adds `HISTORY` when `CHAT` is present), and applied where scope sets are compared as sets:

- `attenuate`: a delegator holding `chat` may mint a `history`-only child. Without this a guest-with-delegate could not hand out a read-only link, which is the point of the scope.
- `effective_scopes`: each capability's stored scopes, and each directory ceiling, are widened with `implied()` before the chain is intersected. Otherwise a `history` child of a `chat` parent, or any capability under a `GUEST` ceiling, would lose the bit in the intersection.
- the directory's `within_ceiling` check for group grants.

Stored bits are not rewritten: a `chat` capability is journaled as `chat`, and `scope_names` lists only bits that are literally present.

## The one projection

`project_event_for(scopes, &event) -> Option<WorldEvent>` replaces `redact_install_seed`:

- a `Chat` event returns `None` unless `scopes` includes `chat` or `history`;
- a `ProgramInstalled` event keeps its module hash and has its seed zeroed (unchanged policy from #240/#250);
- every other event is returned as is.

Callers:

- `WorldCore::events_after` and `WorldCore::snapshot_unchecked` (the viewer's effective scopes are computed once per call);
- the live stream: `WorldCore::project_for` / `WorldHandle::project_event(token, event)`. The session loop used to re-validate the capability on every delivery with a full `snapshot` call; it now asks the actor to project the event, which validates the capability (a revoked participant is still disconnected) and applies the viewer's _current_ scopes (a friendship ending or a revocation between events takes effect on the next event).

`WorldHandle::subscribe` still yields the raw, unprojected event stream (host code and tests need real events, including the real seed). Its doc comment says so; anything that forwards it to a participant must go through `project_event`.

## Gaps

Dropping chat leaves sequence gaps in what a join-only viewer receives. Findings:

- `events_after(cursor)`: `needs_snapshot` and `oldest_retained_sequence` are computed from the retained buffer _before_ projection, so they do not depend on the viewer. `current_sequence` is the world's latest sequence regardless of what was filtered. A client that sets its cursor to `current_sequence` (the session does) or to the last sequence it was given neither loops nor misses a non-chat event: events it was not given are exactly the ones it may not see.
- Snapshot: chat is filtered before the byte-budget loop, so the budget is spent only on events that are included; a join-only viewer gets more non-chat history in the same budget. `oldest_retained_sequence` is the first included event, as before.
- Rust session (`world_session.rs`): the loop compares `event.sequence <= cursor` and then sets `cursor = event.sequence`; it never assumes `+ 1`. A live event that is projected away still advances the cursor so a later duplicate is skipped. No contiguity assumption was found.
- Web client (`web/src/world.ts`): `cursor = Math.max(cursor, event.sequence)` and the rejoin sends `after: cursor`. No contiguity assumption. Not changed.

## Behaviour change

- Existing join-only capabilities (for example children made by `attenuate` with `["join"]`, and the `spectator`-style `join`+`input` sets) stop receiving chat in snapshots, catch-up pages and the live stream. They keep input events and everything else.
- Hosts that want read-only chat must issue or attenuate with `history`.
- The journal format does not change. Scope bits are a number; `history` is `0b1_0000`, unused in every existing journal.
- Older binaries (read from the code, not run): journal replay stores the issued scope bits without validating them (`WorldScopes(scopes)` in `WorldCore::restore`), and `effective_scopes` intersects with the old `ALL` (`0b1111`), so an older binary drops the unknown `history` bit and treats the capability as `join` — which on an older binary still sees all chat, the behaviour this change removes. Directory entries (groups, tiers) go through `WorldScopes::from_bits`, which rejects an unknown bit, so an older binary refuses a directory entry that carries `history` (what its caller does with the refusal was not checked). Over the wire, an older binary's `from_names` returns `None` for `history` and the frame is answered "unknown scope". Downgrading after issuing `history` capabilities is therefore not safe for chat privacy.

## Limits

- The projection is per viewer, not per message: chat sent by the host (`chat.say`, participant 0) is a `Chat` event and is hidden from join-only viewers like any other line.
- The view text a program renders (`View`) and records are not events and are out of scope here; a program that echoes chat into its view shows it to everyone who may see the view.
- `history` does not grant the ability to read chat from before the participant's capability existed beyond what retention keeps; retention is unchanged.

## Non-goals

No change to the journal format, host events (#242), the capability ledger, or the install seed policy beyond routing it through the new projection.

## Follow-ups

- `chat.history` does not exist on `main`. When the roles and `chat.history` work from PR #234 rebases onto this change, `chat.history` must accept `chat` **or** `history`; the `spectator` role stays join-only; a new `observer` role should be `join` + `history`.
- If a client needs to know that lines were withheld, add a count to the page or snapshot (not done: it would tell a join-only viewer how much chat happens).

## Tests

See the `chat_visibility` tests in `src/world.rs` and `src/world_session.rs`:

- join-only: `events_after` and the snapshot omit chat and keep non-chat events; `chat` and `history` participants see chat; a `history`-only participant is refused when sending;
- cursor progress across gaps; the snapshot budget counts only included events;
- the live stream to a join-only session omits chat; a live install event projects to seed 0;
- `from_names(["history"])`, unknown names rejected, attenuation of `history` (including from `chat`);
- through the `Peer` harness: a join-only joiner gets a snapshot without chat while a guest sees the same chat.

## References

- [phase 11 foundations plan](../../.opencode/plans/phase-11-foundations.md) (on another branch)
- [Roc worlds architecture](../2026-10-07T00-20-00Z_architecture_roc_worlds/2026-10-07T00-20-00Z_architecture_roc_worlds.md)
- `src/world.rs`: `WorldScopes`, `project_event_for`, `WorldCore::events_after`, `WorldCore::snapshot_unchecked`
- `src/world_session.rs`: `joined`
