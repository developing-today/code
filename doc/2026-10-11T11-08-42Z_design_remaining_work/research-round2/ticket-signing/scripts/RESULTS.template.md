# RESULTS: signing EndpointTickets

Date: 2026-10-11. Folder: `/home/user/work/research-round2/ticket-signing/`. Test crate: `ticket-options/` (iroh-tickets 1.0.0, iroh-base 1.3.0, postcard 1.1.3). Its `Cargo.lock` was copied from `/home/user/work/spikes/Cargo.lock`. Build target: `target/`, as the parent asked. Written only inside this folder. Read only: the registry crates, `pkgs/id` (source and lock), the spikes lock, S9's RESULTS, and the design docs. No network was used.

Labels: **[verified]** means read in source or seen in output. **[inferred]** means my reasoning from verified facts. **[not checked]** means not tested here.

## Questions (verbatim)

Owner, relayed by the parent:

> n24 i'm not sure what signing tickets entails but we should use or at least allow regular tickets i think and at least for the user they shouldn't need to do anything special they can just validate the signature if they wished. i am on board with signing etc.

Background from the parent: S9 found that an EndpointTicket carries an EndpointId, a relay URL and addresses, has no signature and no expiry, and that trailing bytes are silently ignored by the parser.

1. Read iroh-tickets 1.0.0 in the cargo registry: the EndpointTicket format (prefix, encoding, payload type, the serializer), the parser, and how trailing bytes are handled. Confirm S9's claim in source and with a test.
2. Options, each with a concrete format:
   (A) standard EndpointTicket only, no signature;
   (B) our signed envelope: a new prefix, the standard ticket string inside, and a signature by the inviter's identity key. Standard iroh tools would not parse it;
   (C) standard EndpointTicket, plus a separate signed invite event that carries the signature. The ticket itself stays standard;
   (D) standard EndpointTicket with appended signature bytes that the parser ignores. Test whether this is robust, and whether it changes the prefix or length.
   Also consider (E): both A and C, so standard tickets always work and signatures are optional and checked automatically when present.
3. For each option: what the user copies and pastes, how long the string is, how validation works, what a client does with a bad signature, and compatibility with other iroh tools.
4. Write a small test program that round-trips each option through the iroh-tickets parser and checks that a standard client still accepts the ticket. Show the test output.
5. Recommend one option, and say how it meets the owner's requirements: regular tickets work, signatures are optional, and the user needs to do nothing special.

## Short answers

**1. Format, parser, trailing bytes.** **[verified]** iroh-tickets 1.0.0 defines `EndpointTicket { addr: EndpointAddr }`, with no signature, no expiry and no version field of its own (`src/endpoint.rs:30-32`; `src/lib.rs:14-15` leaves versioning to the implementer). The only wire version marker is the enum variant `Variant1` (`endpoint.rs:34-43`). The payload is postcard: the variant index, the 32-byte EndpointId, and a counted `BTreeSet<TransportAddr>` (`endpoint.rs:129-138`). The string is the prefix `endpoint` plus unpadded base32 of those bytes, lowercased (`lib.rs:44-49`, `endpoint.rs:46, 48-58`). Parsing needs the lowercase prefix (`lib.rs:59`), uppercases the rest for base32 (`lib.rs:62`), and calls `postcard::from_bytes` (`endpoint.rs:61`). postcard documents that the unused part of the input "is not returned" (`postcard-1.1.3/src/de/mod.rs:10-11`). iroh-tickets itself never mentions trailing bytes (grep of its `src` and `README.md`: no matches), so the tolerance is inherited from postcard. S9's three claims hold. The payload carries the EndpointId, relay URLs and IP addresses. There is no signature or expiry field. The parser accepts 0 to 300 appended bytes and returns the same address each time (test `a_trailing_bytes_are_silently_ignored_at_every_length`). It also accepts an edited EndpointId (test `a_swapped_endpoint_id_still_parses_so_a_plain_ticket_proves_nothing`).

**2. The options.** **[verified by tests; the design choices are mine]** Each option is built and round-tripped, and all 19 tests pass. A is the standard string: 124 characters for a relay plus one IPv4 address, the same length S9 measured. B is `idsig` followed by base32 of a postcard envelope that holds the standard ticket string, the inviter's key, and an Ed25519 signature. It is 361 characters, and the standard parser rejects it. C leaves the ticket unchanged and keeps the signature in an invite event (215 bytes in postcard), whose body commits to the BLAKE3 digest of the ticket's canonical bytes. The event is not pasted. D appends a 64-byte signature and the 32-byte signer key after the postcard payload. The prefix stays `endpoint`, the string grows to 277 characters, and the standard parser accepts it with the same address. Its verifier must split by the wire length, because a standard re-share drops the suffix and a non-canonical payload gives the wrong split. E is A plus C: the pasted string is always the standard ticket, and the event is checked when one is present.

**3. What the user pastes, and how each option behaves.** **[verified for lengths, messages and parse results; inferred for the user experience]** The table under Options has the full answer. In short: A, C and E paste the 124-character standard ticket, the format that `pkgs/id` already accepts. B needs a new string of 361 characters that the standard parser rejects, and `pkgs/id`'s `check_target` rejects it too. D pastes 277 characters that the standard parser accepts, but a re-share silently strips the signature. **[inferred]** For B and D the signer key sits inside the string, so the verifier must compare it with a key from outside the ticket; a forger can sign with its own key otherwise. The wrong-signer tests check this. A bad signature is a refusal in B, C and D, with a specific message. In E a missing event is `Unverified`, not an error.

**4. The test program.** **[verified]** `cargo test --offline` in `ticket-options/` runs 19 tests, and all 19 pass (`test-output.txt`). They cover the round trip of each option, standard-parser acceptance for A, C, D and E, the rejection of B by the standard parser, tampering and wrong-signer cases for each signed option, the trailing-byte sweep, the non-canonical split, and the paste rules. A mutation check (`scripts/mutate.py`) makes seven deliberate faults in a scratch copy, and each one fails the test written for it (`mutation-output.txt`). An earlier run reported a red baseline. That was a stale build in the harness's scratch target, because `copytree` kept old mtimes. The harness now clears its target and bumps mtimes, and its baseline is green.

**5. Recommendation.** **[inferred from the measured results]** Recommend **E**. The pasted string stays the standard EndpointTicket, pasted exactly as today. The inviter's signature lives in a signed invite event in the admin's chain, which the design already has (design §10.1). The joiner's client checks the event before it sends the accept. `Verified` means send. `Unverified` (no event reachable) means send with a visible label. `Rejected` means do not dial, and show why. The user does nothing extra, and can inspect the check if they wish. Two conditions make it work. The link must carry the inviter's key as a trust anchor (`by=`). Invite events must be readable before the join (open pull, task 07 item 6).

## Options

### Concrete formats

**A. Standard ticket, unsigned.** `endpoint` + base32(postcard(`Variant1 { id, addrs }`)). Nothing is added.

Example, relay plus one IPv4 address (124 characters, from `print-output.txt`):

```
@@A_EXAMPLE@@
```

**B. Signed envelope.** Prefix `idsig`:

```
"idsig" + base32( postcard( Envelope {
    version:   u8 = 1,
    ticket:    String     = the A string, parsed with EndpointTicket::from_str,
    signer:    [u8; 32]   = the inviter's Ed25519 key, as the envelope claims it,
    signature: [u8; 64]   = Ed25519 verify_strict over b"id-route/v1/envelope" || ticket.encode_bytes()
}) )
```

Decoding is strict, so leftover bytes are an error (`b_rejects_trailing_bytes_that_iroh_would_ignore`). The trust check is `verify_from(K)`. It compares `signer` with K, then checks the signature. Example, 361 characters:

```
@@B_EXAMPLE@@
```

A compact variant would put the payload bytes in place of the string. It is not built. Its length is computed from postcard of the same fields: 277 characters for relay plus one IPv4 address.

**C. Standard ticket plus a signed invite event.** The link becomes `id:join?group=G&invite=I&ticket=T&by=K`. T is the A string, unchanged. K is the inviter's account key, which is 52 base32 characters (measured). `by` is new. The design's link is at `design_groups_events_delegation.md:467`.

The event is stored in the inviter's chain and is not pasted:

```
InviteEvent {
  body: InviteBody {
    group: [u8; 32], invite_id: [u8; 16], expires_ms: u64, max_uses: u32,
    invite_key: [u8; 32],      // the Seitan-derived public key (design §10.1)
    ticket_digest: [u8; 32],   // BLAKE3(b"id-route/v1/ticket-digest" || canonical ticket bytes)
    author: [u8; 32],          // the signing key. It must be K, or a device key delegated from K
  },
  signature: [u8; 64],         // Ed25519 verify_strict over b"id-route/v1/invite-event" || postcard(body)
}
```

The event is 215 bytes in postcard (measured). The design's canonical CBOR would change that figure. The digest is over the canonical bytes of the parsed ticket, not over the string. This matters: the upper-case-body string parses to an equal ticket (asserted in `paste_rules_the_parser_applies_to_the_pasted_string`), so a string digest would differ for one ticket.

**D. Standard ticket plus a suffix.** Prefix `endpoint`, unchanged:

```
"endpoint" + base32( payload || signature[64] || signer[32] )
signature = Ed25519 verify_strict over b"id-route/v1/suffix" || payload
```

The payload is the received postcard bytes. A standard parser ignores the 96-byte suffix. Our split uses a mirror of iroh's private wire type, through `postcard::take_from_bytes`. Example, 277 characters:

```
@@D_EXAMPLE@@
```

**E. A plus C, checked when present.** The pasted string is A. The check is `check_route(ticket_text, Option<&InviteEvent>, K) -> (EndpointTicket, RouteStatus)`:
- `Verified`: the event's author is K, the signature verifies, and the digest matches the pasted ticket.
- `Unverified`: no event is available.
- `Rejected(reason)`: any check fails.

### Comparison

| Option | What the user copies and pastes | Length, relay + 1 IPv4 (measured) | How validation works | Bad signature | Standard iroh tools (`EndpointTicket` parser) |
|---|---|---|---|---|---|
| A | The ticket | 124 | Parse only. The EndpointId is not authenticated, so an edited ID still parses | Nothing to check. An edited ID is accepted silently | Accepts |
| B | One `idsig` envelope | 361 (string inside). 277 (payload inside, computed) | Structure, then `verify_from(K)`: signer must equal K, then Ed25519 `verify_strict` over the domain-tagged ticket bytes | Refuse: `verification failed: bad signature`, or `not signed by the expected inviter` (tested) | Rejects: `wrong prefix, expected endpoint` (tested). `pkgs/id` `check_target` rejects too (tested with the same logic) |
| C | The ticket. The link also carries `by=K` (52 characters). The event is fetched, not pasted | 124 (plus 52 for K in the link) | Event author = K, event signature, and the digest equals the digest of the pasted ticket | Refuse: `event not signed by the expected inviter`, `bad event signature`, or `ticket is not the one the inviter signed` (tested) | Accepts. The ticket is unchanged. The event is not a ticket |
| D | The ticket with a 96-byte suffix | 277 | Split by wire length. Signer = K. Verify over the received payload | Refuse: `bad signature`, `suffix not signed by the expected inviter`, or `trailing bytes are not a signature` (tested). No suffix: `Unverified` | Accepts, same address (tested). A standard re-share drops the suffix (tested) |
| E | The ticket, as today | 124 | As C when an event is present. No event: `Unverified` | `Rejected`: refuse. `Unverified`: proceed with a label (a policy choice, see Recommendation) | Accepts |

### Lengths (measured, `print-output.txt`)

| Address set | A (also C and E pasted) | B, string inside | B, payload inside (computed) | D, signature and key | D, signature only (computed, not built) |
|---|---|---|---|---|---|
| relay only (payload 64 bytes) | 111 | 340 | 265 | 264 | 213 |
| relay + 1 IPv4 (payload 72 bytes) | 124 | 361 | 277 | 277 | 226 |
| relay + 2 IPv4 + 1 IPv6 (payload 100 bytes) | 168 | 433 | 322 | 322 | 271 |

C's event is 215 bytes, which is 344 base32 characters if it were carried inline (computed). A 32-byte key is 52 base32 characters (computed by `data-encoding`'s `encode_len`).

### Paste rules (measured, shared by every option)

| Paste | Result |
|---|---|
| exact copy | accepted |
| trailing newline | rejected: `invalid symbol at 116` |
| trailing space | rejected: `invalid symbol at 116` |
| leading space | rejected: `wrong prefix, expected endpoint` |
| upper-case prefix (`ENDPOINT…`) | rejected: `wrong prefix, expected endpoint` |
| upper-case body | accepted, and equal to the exact copy |
| B trailing newline | rejected |
| D trailing newline | rejected |

## Recommendation

**Recommend E: the standard ticket, plus an inviter-signed invite event that commits to the ticket's digest.** It uses pieces the design already has:
- The link gains `by=K`. The design's link format is at `design_groups_events_delegation.md:467`.
- The invite event (task 07 item 3) gains `ticket_digest`. Events are already signed by their author (design line 31, and line 78 on `author`).
- The joiner checks the event before sending the accept. `Verified`: send. `Unverified`: send, with a visible "route not verified" label. `Rejected`: do not dial, and show the reason.

### How E meets the owner's requirements

- **Regular tickets work.** The pasted string is the standard ticket: 124 characters for relay plus one IPv4 address. iroh-tickets 1.0.0 accepts it with the same address (tests `a_is_the_standard_string_and_round_trips`, `c_verifies_the_unchanged_standard_ticket_against_the_signed_event`, `e_a_plain_ticket_is_accepted_and_unverified_not_refused`). `check_target` (`pkgs/id/src/envelope_outbox.rs:394-404`) needs no change. **[verified]**
- **The user does nothing special.** The user pastes what they paste today. The check runs in the client. **[inferred]**
- **Validation is optional for the user.** The client shows the status. The inviter's key K can be shown as a fingerprint for a manual comparison if the user wants one. **[inferred]**
- **Signing is in.** The invite event is signed by the inviter's key, and the ticket is bound to it by digest. Wrong signers, changed tickets, swapped IDs and forged events are all refused (tests `c_*` and `e_*`). **[verified]**

### Why not the others

- **A alone.** An edited EndpointId parses, and nothing flags it (test `a_swapped_endpoint_id…`). It meets the paste requirement but not the signing one. **[verified]**
- **B.** Standard tools reject it (`wrong prefix, expected endpoint`). `pkgs/id`'s `check_target` rejects it too. It is the longest paste, at 361 characters. Its one advantage is a self-contained string, which matters only for offline verification. **[verified for the rejection; the advantage is inferred]**
- **D.** It is shorter than B (277 characters) and keeps the prefix. But a standard re-share silently strips the signature (test `d_is_dropped_when_a_standard_client_reads_and_writes_the_ticket`). A split by re-encoding breaks on a non-canonical payload: 94 bytes on the wire, 64 when re-encoded (test `d_split_follows_the_wire_length…`). It also depends on postcard's `from_bytes` tolerance, which iroh-tickets does not state. A future strict parser would reject every D ticket, which breaks the standard-client guarantee the owner asked for. **[verified for the first two; inferred for the last]**

### Join flow under E (proposal)

1. Trim the pasted text. A trailing newline or space fails the parser (measured). `check_target` does not trim (`envelope_outbox.rs:394-404`; its callers at 172, 183 and 376 pass the raw target). **[verified]**
2. Parse the ticket with the standard parser.
3. Ask the ticket's node for the invite event by invite ID, before sending the accept. Open pull allows this (task 07 item 6). **[inferred]**
4. Run `check_route`. `Verified` means send the accept. `Unverified` means send it with the label. `Rejected` means stop.

### Decisions for the owner

1. **Unverified policy.** Proceed with a label (my recommendation), or refuse. Proceeding is reasonable because the Seitan proof still guards admission, so a bad route can only leak metadata or stall the join. **[inferred]**
2. **The trust anchor K.** A signature proves consistency with K only. Whoever writes the link chooses K, which is the same trust the link has today. **[inferred]**
3. **Invite events must be readable before joining.** Under authorized pull, E returns `Unverified` for every invite unless the event is served by invite ID. This needs a decision in task 07 item 6.

### What would change the recommendation

- **Offline verification** (a QR code, paste only, no network before the join). Carry the event inline with `by=K`: about 344 characters for the event plus 52 for the key (computed). That avoids D's dependence on parser tolerance. D is shorter (+153 characters over A) but riskier.
- **Authorized pull as the default for groups.** E cannot verify an invite before the join.
- **A requirement to bind group, invite and expiry in one string.** Use B, alongside A, not in its place. Adding the group ID and invite ID would add 48 bytes, about 77 characters. That is an estimate and was not built.
- **iroh-tickets adding a trailing-byte check.** E and C would not change. D would break.

### What changes in the design, if E is adopted

- Task 07 item 3 (invite event): add `ticket_digest` to the body.
- Design §10.3 (link): add `by=K`.
- Design §10.3 step 2: fetch and check the event before the accept.
- `pkgs/id`: trim the pasted target. No change to the EndpointTicket parse.

## Evidence

### Source (read directly)

| Claim | Location |
|---|---|
| `EndpointTicket { addr: EndpointAddr }`; no signature, expiry or version field | `iroh-tickets-1.0.0/src/endpoint.rs:30-32` |
| Versioning is left to the implementer | `iroh-tickets-1.0.0/src/lib.rs:14-15` |
| Wire format: one variant `Variant1`, holding `id` and `info.addrs` | `endpoint.rs:34-43, 129-138` |
| `KIND = "endpoint"` | `endpoint.rs:46` |
| Serializer: `postcard::to_stdvec` of the wire enum | `endpoint.rs:48-58` |
| Parser: `postcard::from_bytes(bytes)?` | `endpoint.rs:60-69` (the call is at 61) |
| `FromStr` calls `Ticket::decode_string` | `endpoint.rs:75-77` |
| String form: KIND plus `BASE32_NOPAD.encode`, lowercased | `lib.rs:44-49` |
| Decoding: case-sensitive `strip_prefix` (59); base32 of the uppercased body (62) | `lib.rs:57-64` |
| Error variants, including `Verify { message }`, and the `verification_failed` constructor | `lib.rs:71-93, 106-108` |
| postcard `from_bytes`: "The unused portion (if any) of the byte slice is not returned" | `postcard-1.1.3/src/de/mod.rs:10-19` |
| postcard `take_from_bytes` returns the unused portion | `postcard-1.1.3/src/de/mod.rs:60-70` |
| `EndpointAddr { id, addrs: BTreeSet<TransportAddr> }` | `iroh-base-1.3.0/src/endpoint_addr.rs:41-47` |
| `TransportAddr` is `#[non_exhaustive]`: Relay, Ip, Custom | `endpoint_addr.rs:54-62` |
| `PublicKey::verify` uses `verify_strict` | `iroh-base-1.3.0/src/key.rs:134-138` |
| `PublicKey` `FromStr` uses base32-hex | `key.rs:249-256` |
| iroh-tickets does not mention trailing bytes (grep of `src` and `README.md`) | no matches |

### pkgs/id (read only)

| Claim | Location |
|---|---|
| `check_target` tries `EndpointTicket` first, then `EndpointId`. No trimming | `pkgs/id/src/envelope_outbox.rs:394-404` |
| The only `EndpointTicket` parse in `pkgs/id/src` (grep). Line 18 is the import, and 443 is a test | `envelope_outbox.rs:395` |
| Callers pass the raw target: `enqueue`, `enqueue_artifact`, and delivery | `envelope_outbox.rs:172, 183, 376` |
| Pinned versions: iroh-tickets 1.0.0, iroh-base 1.3.0, postcard 1.1.3, ed25519-dalek 3.0.0, iroh 1.3.0 | `pkgs/id/Cargo.lock:3339, 3085, 4998, 1683, 3035` |

### Prior work and design (read only)

| Claim | Location |
|---|---|
| S9: the ticket probe (124 characters; 18 bytes appended parse; EndpointId swap parses) | `/home/user/work/spikes-next/s9-seitan/RESULTS.md:8, 125-138` |
| S9 verdicts: a token cannot ride in a ticket; expiry and single use belong to the invite event | `RESULTS.md:144-154` |
| Events are signed. `author` is the signing key | `design_groups_events_delegation.md:31, 78` |
| Invite event contents (invite ID, expiry, public key); the link format | `design_groups_events_delegation.md` §10.1 and line 467 |
| Invite event, tickets, pull policy | `task-07-invites-and-pull.md:21, 23, 24` |

### Commands and environment

```
rustup which --toolchain 1.97.0 cargo
  -> /home/user/.rustup/toolchains/1.97.0-x86_64-unknown-linux-gnu/bin/cargo
export PATH=/home/user/.rustup/toolchains/1.97.0-x86_64-unknown-linux-gnu/bin:$PATH
export RUSTC=/home/user/.rustup/toolchains/1.97.0-x86_64-unknown-linux-gnu/bin/rustc
export CARGO_TARGET_DIR=/home/user/work/research-round2/ticket-signing/target
cd ticket-options
cargo test --offline                                                   # -> test-output.txt
cargo test --offline --test options -- --nocapture --test-threads=1   # -> print-output.txt
cd .. && python3 -I scripts/mutate.py                                  # -> mutation-output.txt, mutation-logs/
python3 -I scripts/render_results.py                                   # -> RESULTS.md
```

Observed: `rustc 1.97.0 (2d8144b78 2026-07-07)` and `cargo 1.97.0 (c980f4866 2026-06-30)`. The build used `--offline`, and it succeeded, so every crate came from the local registry. The build compiled iroh-base 1.3.0 and iroh-tickets 1.0.0, and their dependencies, from source. No build touched `/home/user/code`, `integrated`, `spikes`, or `spikes-next`.

### Test output (`cargo test --offline`)

```
@@TEST_OUTPUT@@
```

### Test output with printed strings and lengths (`--nocapture`)

```
@@PRINT_OUTPUT@@
```

### Mutation check (`scripts/mutate.py`)

Each mutant changes one line of `src/lib.rs` in a scratch copy, with its own target directory. The real crate is never edited.

```
@@MUTATION_OUTPUT@@
```

| # | Fault | Failing test(s) |
|---|---|---|
| baseline | none | none (19 passed) |
| M1 | B accepts trailing bytes | `b_rejects_trailing_bytes_that_iroh_would_ignore` |
| M2 | D skips the expected-signer check | `d_verifies_a_valid_suffix_and_rejects_each_tamper` |
| M3 | C skips the digest check | `c_rejects_a_swapped_endpoint_id…`, `e_a_valid_event_verifies…` |
| M4 | D splits by re-encoding | `d_split_follows_the_wire_length…` |
| M5 | B and D sign with no domain tag | `a_signature_made_for_option_d_is_refused_as_option_b` |
| M6 | E refuses a plain ticket | `e_a_plain_ticket_is_accepted_and_unverified_not_refused` |
| M7 | C ignores the event's author | `c_rejects_an_event_from_another_key…` |

### Files

```
ticket-signing/
  RESULTS.md                 this file (generated)
  ticket-options/            crate: src/lib.rs (options A to E), tests/options.rs (19 tests), Cargo.lock
  scripts/mutate.py          mutation check (scratch copy, its own target dir)
  scripts/render_results.py  renders RESULTS.md from scripts/RESULTS.template.md and the logs
  scripts/RESULTS.template.md
  test-output.txt            cargo test (19 passed)
  print-output.txt           lengths, examples, paste rules
  mutation-output.txt        mutation summary
  mutation-logs/             full cargo output for each mutant
  target/                    build directory
```

## Not verified

- **No live dial or connection.** I did not test whether iroh's QUIC handshake authenticates the EndpointId. Nothing above depends on it, except the claim that an edited ID changes the route, which holds at the ticket layer. **[not checked]**
- **Only iroh-tickets 1.0.0, iroh-base 1.3.0 and postcard 1.1.3 were used.** I did not check later versions, and I did not check whether postcard or iroh-tickets will keep trailing-byte tolerance. **[not checked]**
- **Event encoding.** The tests use postcard. The design's canonical CBOR (task 02) would change the 215-byte figure. I did not test the canonical re-encoding rule for events. **[not checked]**
- **Delegation.** The event's author may be a device key delegated from the account key (design §7). I did not test that chain. **[not checked]**
- **B's group and invite binding.** Not built, so its cost (about 77 characters) is an estimate. **[not checked]**
- **Relay URL normalization, and `TransportAddr::Custom`.** Not tested. The digest's stability is tested only for the sample address. **[not checked]**
- **Non-minimal varints.** Not tested. The duplicate-address test covers a different kind of non-canonical input. **[not checked]**
- **Callers of `check_target`.** I read the call sites in `envelope_outbox.rs` only. The web and CLI callers were not read, so I cannot say whether they trim. **[not checked]**
- **Compact B length.** Computed from postcard serialization of the same fields. It was not parsed. **[inferred]**
- **Expiry and single use.** No signature here covers them. They belong to the invite event, as in task 07 item 3. **[not checked]**
- **The validation UX** (the status labels and the fingerprint display) is a proposal, not tested. **[not checked]**
- **Seitan.** Not involved here. S9's results are cited, not re-verified. **[not checked]**
- **`pkgs/id` was not changed or built.** Its behaviour is read from source, not run. **[not checked]**
