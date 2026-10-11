# Task 07: Invitations and pull privacy

- Status: Designed. The Seitan derivations are verified against source (research §8.1). Implementation needs S9 (test vector and admin path). The invite style, approval, and pull defaults need decisions.
- Index: [what is left](./2026-10-11T11-08-42Z_design_remaining_work.md). Depends on: [task 02](./task-02-events-and-storage.md) and [task 03](./task-03-membership-and-delegation.md).
- Design: [design §10 and §9.4–§10.3](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md), [connectivity §2](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T10-46-58Z_design_connectivity.md). Research: [§8 and §9](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T04-20-17Z_research_findings.md).

## Goal

A join needs two things: an authority (the Seitan key, which only the invitee holds) and a route (an iroh ticket, which only gives addresses). The pull policy and what it reveals are documented plainly.

## Scope

1. **Seitan V2.** From keybase/client `go/teams/seitan.go`, verified:
   - The token is 17 random characters from `abcdefghjkmnpqrsuvwxyz23456789` (about 83 bits), plus a `+` at index 6 in the full token.
   - `siKey` = scrypt(lowercased token, empty salt, N = 2^10, r = 8, p = 1, 32 bytes).
   - Invite ID = the first 15 bytes of HMAC-SHA512 over `msgpack{stage: "invite_id", version: 2}`, followed by a tag byte.
   - Ed25519 seed = the first 32 bytes of HMAC-SHA512 over `msgpack{stage: "eddsa", version: 2}`.
   - Accept message = `msgpack{stage: "accept", uid, eldest_seqno, ctime, invite_id, version: 2}`, signed with the derived key.
   - Test vector: token `4uywza+b3cga7rd6yc` gives invite ID `6303ec43bd61d21edb95a433faf06227`.
2. **Freshness.** Add a check on `ctime` in the admin path. Keybase's V2 lacks it, and its V3 has it.
3. **Invite event.** A signed event in the admin's chain: the invite ID, the invite's public key, an expiry, an optional use limit, and a revoke. Optional approval. The public key may be published in the clear, since knowing it does not let anyone sign an accept.
4. **Links.** `id:join?group=…&invite=…&ticket=…`. The Seitan key is typed in, or sits in the URL fragment for a bearer link. Keybase's invite links use the same fragment pattern.
5. **Tickets.** An `EndpointTicket` gives addresses only. It grants nothing, and it has no expiry (the invite event carries the expiry).
6. **Pull policy.** Open by default, with an `authorized` option. Authorized pull is advisory: it stops honest nodes from serving unauthorised peers. It does not stop re-serving, does not recall copies, and does not hide metadata.
7. **Metadata documentation.** Any storing node can read: author key, sequence number, kind, group ID, `after` hashes, sizes, and timing. Bodies are encrypted in end-to-end groups and readable in `scheme: none` groups. This goes into user-facing docs.
8. **Replacing today's invite links.** `invite.rs` sets petnames and does not join anything. Keep that behaviour. It can carry a ticket later.

## Decisions

- Bearer link or typed key, or both. Approval on or off. The default expiry.
- Pull default: open (decided). Confirm the authorized option's name.

## Acceptance

- S9: the derivations reproduce the test vector. The accept message round-trips. The admin path rejects a stale `ctime`, a used invite, an expired one, and a revoked one.
- A bearer link works, and a typed link fails without the key.
- Metadata documentation matches what the code exposes (checked by a list in the test).

## Gotchas

- The Keybase book page returned 403 to our fetcher, so the parameters come from the source. Verify against the source again if Keybase changes.
- Scrypt N = 2^10 is cheap. That is acceptable for random 83-bit tokens, and weak for guessable ones. Never use a user-chosen token.
- localfirst/auth stretches with a fixed-key BLAKE2b, not scrypt, despite its comments. Do not copy its stretch.

## Files

New module, for example `pkgs/id/src/invite/`. `invite.rs` stays for petnames until then.

## Dependencies

`ed25519-dalek` (already present), `sha2` (already present), `hmac`, `scrypt` (transitive), `rmp-serde` (already present). Report each new one.

## 2026-10-11T12-20-38Z Update: invites and pull (S9 running)

- S9 is running in `spikes-next/s9-seitan`: Seitan V2 derivations against the reference code, the freshness check, and the admin path.
- Open from this session: can a Seitan token ride inside an `EndpointTicket`? That depends on what the ticket leaks and whether tickets expire. S9 reports it.
- Invite style (#10): preferred is a bearer link with expiry and single use. A typed key stays optional, with owner approval.
- Pull default (#11): open pull, as decided, with an authorized option. Not yet confirmed by the owner.

## 2026-10-11T12-49-09Z Update: Seitan V2 (S9)

- Derivations match Keybase's own V2 vectors (`go/teams/seitan_v2_test.go`): the scrypt stretch, the invite ID, and the Ed25519 public key. The public key check opened the vector's key, so it is end to end.
- Not verified: the accept-message bytes (no vector covers them), the Keybase admin handler and V3 (guessed paths returned 404), and kbcrypto's signing call.
- Admin path: 38 of 38 tests pass, and clippy is clean at `-D warnings` for that crate. Single use, expiry, revocation, replay and forgery are tested. All 10 deliberate mutations are caught.
- Freshness, a decision: bound the accept time `ctime` to the invite's lifetime: `created − 5 min ≤ ctime ≤ min(now + 5 min, expires)`. A wall-clock window would reject accepts that a home server holds, which the design allows.
- A Seitan token cannot ride inside an `EndpointTicket`: the format has no field for it. Bytes appended to the payload do parse, because postcard ignores trailing data, and iroh then drops them without an error.
- What a ticket carries: the EndpointId, the home relay URL, and any IP:port. A ticket has no expiry and no signature. Expiry and single use belong to the admin's invite event.
- Gotcha: `seitan.go` still has the V1 invite-ID payload (no version) and the V1 `+` position. Implementing from it alone gives the wrong V2 ID.
- The research note says scrypt is transitive. It is absent from the spike's lock.
- Deviation: the spike used ed25519-dalek 3, to match `pkgs/id`.
