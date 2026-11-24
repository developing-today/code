# Nostr — full research: how it works, whether it is still used, and its fit as a transport adapter

*Research record, 2026-11-23 (lane NOSTR-RESEARCH, at Drew's direction). Primary sources are the NIPs themselves (read from `nostr-protocol/nips` at 2026-11-24) and live repository/registry data gathered the same day; secondary sources are named where used. Part 3 assesses Nostr against our own multi-transport adapter contract (#13929) and epic (#13926).*

**Verdicts up front**

- **Liveness: niche-but-stable.** Social growth flatlined after the 2023 peak — Nostr never reached 100,000 weekly active users in any week of 2025 — but the protocol is not dead: the relay population is not shrinking, the main clients and the NIPs repo were all pushed to within the last week, OpenSats' Nostr Fund has allocated over $10M since 2023 and ran four grant waves in Dec 2025–May 2026, and the centre of gravity is visibly shifting from social feeds to Nostr-as-infrastructure (MLS messaging, a production P2P exchange, wallet RPC, MCP transport).
- **Adapter fit: an honest eventual transport adapter at read + bidirectional, write-capable with receipt-time capability verification — and explicitly not sync-equivalent.** Its natively asymmetric relay model (NIP-65 separates read and write relays per identity) fits the contract's per-direction levels unusually well. Its hard limits are equally clear: ~128 KiB practical payload ceiling on public relays, no revocation or key rotation anywhere in the protocol, and relay-visible metadata.

---

## Part 1 — How Nostr works

### 1.1 The one object: the event

Nostr ("Notes and Other Stuff Transmitted by Relays") has exactly one data structure, defined by NIP-01, which is mandatory for everyone:

```json
{
  "id":        "<32-byte lowercase hex sha256 of the serialized event>",
  "pubkey":    "<32-byte lowercase hex public key of the event creator>",
  "created_at": "<unix timestamp in seconds>",
  "kind":      "<integer>",
  "tags":      [["<tag name>", "<tag value>", "..."], "..."],
  "content":   "<arbitrary string>",
  "sig":       "<64-byte lowercase hex Schnorr signature of the id>"
}
```

The `id` is the SHA-256 of a canonical JSON serialization `[0, pubkey, created_at, kind, tags, content]` (no whitespace). The signature is a BIP-340 Schnorr signature over secp256k1 — the Bitcoin curve and signature scheme. There is no other object type, no global database, no consensus, and no blockchain. Everything — posts, profiles, follow lists, encrypted messages, orders, wallet commands — is an event of some kind.

Tags are arrays of strings. Single-letter tag names (`e` for event references, `p` for pubkey references, `a` for addressable-event references, `d` for the addressable identifier) are indexed by relays by convention, which is what makes filtered queries possible.

### 1.2 Identity: a bare keypair, nothing else

An identity is a secp256k1 keypair. The public key is the identity; the private key is the whole account. NIP-19 defines bech32 display encodings (`npub1…` for public keys, `nsec1…` for private keys, `note1…`, `nprofile1…`, `nevent1…` for references). There is no registration, no recovery, no rotation, and no revocation: lose the private key and the identity is gone; leak it and the identity is someone else's, permanently and undetectably. This single fact drives most of the failure model in §1.8 and most of the adapter caveats in Part 3.

Key management has accreted mitigations rather than fixes: NIP-07 (browser-extension signing), NIP-46 (remote signing / "bunkers" — the key stays on a signer device and apps ask it to sign), and NIP-26 (delegated signing tokens) exist; NIP-46 is the one with real adoption. The ecosystem's working answer to key risk is *key compartmentalisation* — separate keys per purpose (see NWC and Mostro in §1.7) — not hierarchy.

### 1.3 Kinds: one integer, four storage classes

The `kind` integer determines both meaning and how relays must treat the event (NIP-01):

| Range | Class | Relay behaviour |
|---|---|---|
| 0, 3, 10000–19999 | Replaceable | Only the latest event per (pubkey, kind) is kept |
| 30000–39999 | Addressable (parameterized replaceable) | Only the latest per (pubkey, kind, `d` tag) is kept |
| 20000–29999 | Ephemeral | Forwarded to current subscribers, not stored |
| everything else (1, 2, 4–44, 1000–9999, …) | Regular | Stored; all versions kept |

Load-bearing kinds: `0` profile metadata, `1` text note, `3` follow list, `5` deletion request (NIP-09), `7` reaction, `14` chat message (used as the NIP-17 rumor), `10002` relay list (NIP-65), `1059` gift wrap (NIP-59), `13194`/`23194`/`23195` Nostr Wallet Connect info/request/response (NIP-47), `30023` long-form article. Kinds are a commons: any NIP or app can allocate them, and collisions are resolved socially through the NIPs repo.

### 1.4 Relays and the wire protocol

Clients talk to relays over a single WebSocket connection per relay. The entire client↔relay protocol (NIP-01) is:

- Client → relay: `["EVENT", <event>]` (publish), `["REQ", <subscription id>, <filter>, …]` (subscribe), `["CLOSE", <subscription id>]`, `["AUTH", <signed event>]` (NIP-42).
- Relay → client: `["EVENT", <subscription id>, <event>]`, `["EOSE", <subscription id>]` (end of stored events; live events follow), `["OK", <event id>, <bool>, <message>]` (publish acknowledgement), `["CLOSED", …]`, `["NOTICE", <message>]`, `["AUTH", <challenge>]`.

A **filter** is a JSON object: `ids`, `authors`, `kinds`, `#<single-letter tag>` arrays, `since`/`until` timestamps, `limit`, and `search` on relays that support it. A subscription matches the union of its filters. This is the whole query language — deliberately. There is no server-side computation beyond filter matching and (on some relays) NIP-45 `COUNT`.

Relays are independent and unopinionated by design. A relay may accept only some users, charge, require NIP-42 authentication, rate-limit, index only some kinds, prune aggressively, or delete old events. NIP-11 defines a relay information document served over HTTPS at the relay URL (`name`, `description`, `supported_nips`, `software`, `version`, and a `limitation` object: `max_message_length`, `max_content_length`, `max_subscriptions`, `max_limit`, `min_pow_difficulty`, `auth_required`, `payment_required`, …). Clients are expected to read policies off this document and adapt. Nothing guarantees two relays hold the same data; the client's job is to query enough relays.

Relay implementations in real use: strfry (C++, the common public-relay default), nostr-rs-relay, khatru and others. Relay *monitors* (NIP-66) publish signed relay-health reports as ordinary events (kind 30166), which is how directories like nostr.watch measure the population — this matters for Part 2's evidence quality.

### 1.5 The NIPs system: a small core, a long tail

Protocol evolution is by NIPs (Nostr Implementation Possibilities) — markdown documents in one GitHub repo, public domain, each marked `draft`/`final` and `optional`/`mandatory`/`unrecommended`. There are well over a hundred. Interoperability in practice rests on a small core:

| NIP | Subject | Why it matters |
|---|---|---|
| 01 | Events, signatures, relay protocol, filters | The protocol itself; mandatory |
| 02 | Follow lists (kind 3) | The social graph primitive |
| 05 | DNS-based identifiers | Human-readable names via `/.well-known/nostr.json` (§1.6) |
| 09 | Deletion requests (kind 5) | The only deletion mechanism — a request (§1.8) |
| 10 | Threading conventions | `e`-tag reply/mention discipline for text notes |
| 11 | Relay information document | How clients learn relay policy and limits |
| 13 | Proof of work | Spam cost as an event property |
| 17 | Private direct messages | The current DM standard (44 + 59 composition) |
| 19 | bech32 encodings | `npub`/`nsec`/`nevent` human forms |
| 42 | Client→relay authentication | Signed challenge-response; gates paid/private relays |
| 44 | Versioned encrypted payloads | The current encryption primitive (§1.6) |
| 47 | Nostr Wallet Connect | RPC to a Lightning wallet over relays (§1.7) |
| 57 | Zaps | Lightning payment receipts as events |
| 59 | Gift wrap | Metadata-hiding event encapsulation (§1.6) |
| 65 | Relay list metadata / outbox model | Per-identity read/write relay discovery (§1.6) |
| 66 | Relay monitoring | Signed relay-health reports; the measurement substrate |
| 70 | Protected events | `["-"]` tag: relays must not accept republication by non-authors |
| 77 | Negentropy sync | Set reconciliation between a client and one relay (§3.4) |

The long tail (markets, groups (29), live streaming, badges, calendars, long-form (23), file metadata (94), communities, DVMs (90), …) is where application families live. Two status markings are themselves evidence of a functioning standards culture: NIP-04 (the original DM scheme) is marked **unrecommended**, and NIP-EE (MLS messaging, merged Aug 2025) was marked **final but unrecommended** in Dec 2025 when the standalone Marmot protocol superseded it — the registry corrects itself in public.

### 1.6 Encryption and discovery layers

**Encryption, three generations.**

1. **NIP-04 (legacy, unrecommended):** AES-256-CBC over a raw ECDH shared secret, no padding discipline worth the name, and — worst — fully public metadata: sender, recipient, and timing are in the clear event (kind 4). Still supported by old clients; nobody should build on it.
2. **NIP-44 (current primitive):** *versioned* encrypted payloads. Version 2: ECDH conversation key → HKDF → ChaCha stream with HMAC-SHA256 authentication, length-hiding padding, base64 armour. It encrypts a payload from one keypair to another — nothing more; composition is up to the layers above.
3. **NIP-59 gift wrap + NIP-17 DMs (current messaging):** three layers — a **rumor** (the real message, unsigned, deniable), a **seal** (kind 13: the rumor NIP-44-encrypted to the recipient and signed by the real sender; reveals the sender, hides content and recipient), and a **gift wrap** (kind 1059: the seal encrypted again and signed by a *fresh ephemeral keypair*, addressed only by a `p` tag; reveals the recipient, hides the sender). `created_at` on the outer layers is randomized up to two days into the past to break timing correlation. A relay watching gift wraps learns that *someone* sent *something* to a given pubkey at approximately some time — and nothing else. NIP-17 builds private DMs (kinds 14/15 rumors) on this. Mostro's first protocol version and much else use the same composition.
4. **Marmot / MLS (group messaging, the new work):** for groups, the ecosystem is moving to MLS (IETF RFC 9420) with Nostr as identity + delivery service and Blossom for encrypted blobs: continuous group key agreement, forward secrecy, post-compromise security. White Noise is the flagship implementation (rebuilt 2026; native apps in beta since Sep 2026), signing chat messages with one-time keys so relays see a random group id and ciphertext. Nostr here is explicitly a *transport and identity layer for a protocol designed elsewhere* — a pattern Part 3 returns to.

**Discovery.**

- **NIP-05:** a kind-0 event may claim `name@domain`; a client verifies by fetching `https://domain/.well-known/nostr.json?name=<name>`, which maps the name to a pubkey (and may advertise that key's relays). The spec itself is blunt about the trust shape: clients must follow *pubkeys, not NIP-05 addresses* — if the domain later returns a different key, the client must stop displaying the name, not switch keys. NIP-05 is a naming/attestation convenience anchored in DNS and domain control, not an identity root.
- **NIP-65, the outbox model:** each user publishes a replaceable kind-10002 event listing their **read** relays and **write** relays separately. To fetch a user's events, query *their write relays*; to deliver mentions to them, publish to *their read relays*. This is the protocol's answer to "where does this identity live" without any central directory — and it is natively, per-direction asymmetric, which matters in Part 3.

### 1.7 Nostr as a transport for non-social applications

The strongest evidence that Nostr is a general signed-event transport, not just a social protocol:

- **Nostr Wallet Connect (NIP-47):** a client sends an encrypted request event (kind 23194) to a wallet service listening on agreed relays; the service executes against the user's Lightning wallet and answers with an encrypted response (kind 23195). Per-connection keys, deliberately unlinkable to the user's social identity. This is production RPC-over-relays, in wallets today.
- **Mostro:** a non-custodial Lightning P2P exchange daemon. Orders are published as events (it contributed the NIP-69 P2P order standard upstream); trade coordination runs as encrypted direct messages (protocol v2: signed kind-14 with NIP-44; v1 used NIP-59 gift wrap and was removed in v0.19.0); clients rotate trade keys per order to break linkability; escrow is by Lightning hold invoices, so the Nostr layer carries coordination, never funds. It is at v0.20.1 (Oct 2026), 50 releases, OpenSats-funded (2024, renewed 2025), with real usage concentrated in Latin America.
- **ContextVM:** an MCP-over-Nostr protocol — servers and clients communicate over relays — funded in OpenSats' 15th Nostr grant wave (Feb 2026) for "agentic AI tooling". Directly relevant to our own MCP door: the pattern of running a tool protocol over Nostr already exists and is funded.
- **The rest of the infrastructure shelf:** Zapstore (an app store whose packages are signed and distributed as events, installed by key), Nostr Mail (a complete email system on npubs, OpenSats wave 16), Routstr (pay-per-request AI API access over Nostr + Cashu), TollGate (WiFi routers selling internet access, coordinated over Nostr), Joinstr (decentralised CoinJoin coordination), and Blossom (the companion blob layer: media lives on Blossom servers addressed by hash, events carry references — Nostr events are deliberately not the blob store).

### 1.8 Trust and failure model

- **Relay trust is availability + metadata, never content.** A relay cannot forge or alter an event (the signature fails), and cannot read gift-wrapped content. It *can* see connection IPs, subscription filters (which pubkeys and kinds a client cares about — a real interest leak), timing, and sizes; it can censor, delay, withhold, or silently drop. The protocol's mitigation is structural: publish to several relays, read from several relays (the outbox model), and treat any single relay's view as partial. NIP-70 protected events and NIP-42 auth let relays enforce author-only publication for marked events, but enforcement is per-relay.
- **No global state.** There is no "the network's copy" of anything. Every relay holds a policy-shaped subset; every client assembles its own view. Convergence is eventual, best-effort, and unverifiable in general — the exact opposite of a ledger.
- **Deletion is a request.** NIP-09 kind-5 events ask relays to delete referenced events; relays SHOULD comply, clients SHOULD hide, and the deletion request itself is broadcast and retained indefinitely. Any relay (or any client that already fetched the event) may keep the content forever. NIP-40 expiration timestamps are similarly advisory per relay.
- **Replaceable events discard history by design.** For kinds 0/3/10000–19999 and addressables, relays keep only the latest; earlier states are gone from compliant relays. Convenient for profiles and lists; fatal to any use that needs an audit trail from the relay layer.
- **Key compromise is total and permanent.** No rotation, no revocation list, no recovery. §1.2's compartmentalisation (per-purpose keys, remote signers) is the only defence, and it is convention, not protocol.
- **Spam is the tax on open writes.** Free public relays attract floods — including industrial-scale junk-key generation that poisons every naive metric (Part 2). Countermeasures in use: NIP-13 proof-of-work, paid relays, allowlists, NIP-42-gated writes, web-of-trust filtering in clients. None is protocol-mandatory.
- **Censorship resistance: real at the identity layer, weaker at the distribution layer.** Nobody can take your keypair or stop you publishing to *some* relay, and your audience can find you at your write relays wherever they are. But discovery and attention concentrate: in the 2023 academic measurement the top relay hosted 73% of observed posts, and NIP-65 lists in the wild are dominated by a few defaults (relay.primal.net appears in ~783K lists — largely a default-onboarding artifact; relay.mostr.pub's ~673K is a Mastodon bridge's footprint, not organic users). The protocol resists censorship; the *attention economy on top of it* centralises anyway.

---

## Part 2 — Is it still used? (evidence, 2025–2026)

### 2.1 Why Nostr metrics lie, stated once

Three traps recur in every Nostr statistic. (1) **Pubkey counts are meaningless**: keypairs are free, spam operations mint them industrially — one dashboard reported "28.84M publishing users last week" (Nov 2026), which is junk-key flood traffic, not people. (2) **Relay "user" counts are default artifacts**: NIP-65 lists copied from client defaults and bridge relays (relay.mostr.pub is a Mastodon bridge) inflate apparent adoption by hundreds of thousands. (3) **Every dashboard sees a different network**: there is no global state (§1.8), so each stats site reports the slice its relays carry. The evidence below therefore leans on the hardest-to-game signals: relay monitors with a fixed methodology over two years, repository activity, grant records, and a prediction market that resolved against the canonical stats source.

### 2.2 The evidence

| Signal | Measurement | Date / source |
|---|---|---|
| Relays answering monitors | 786–787 (Jul 2024) → 1,119–1,139 across the same five nostr.watch monitors; 1,207 addresses answered at least one of 8 monitors in the latest round | 23 Sep 2026, nostr.co.uk's two-year NIP-66 monitor analysis |
| Relay churn | Only 32% of relays answering in Jul 2024 still answered in Sep 2026 — high turnover, stable-to-growing population | same |
| Relay census (academic) | 3,168 observable relays (incl. intermittent), 640M events, 46M pubkeys observed, ~1.2M pubkeys with profiles + contact lists ("identities") | Nov 2025 telemetry, IFAC 2026 preprint (NTNU) |
| Weekly active users | **Never reached 100,000 in any week of 2025** (Manifold market on stats.nostr.band WAU resolved NO); late-2025 samples put active writers at ~80,000 pubkeys | 2025, via Punk Science tech brief + market record |
| Advocate concession | "Activity on Nostr has not only flatlined, but may have even declined… stuck at the same level it was 12 months ago" — Aleksandar Svetski, Nostr advocate | 2025, quoted in Punk Science |
| Small-slice user counts | One Oct 2025 dashboard slice: 21,281 users, 3,675 daily active (method unclear — treat as a floor sample, not a census); per-client weekly users by client-tagged events: Amethyst ~2,646, Primal ~2,441, Ditto ~600 (nostrhub) | Oct–Nov 2025/2026 dashboards |
| Retention | Studies cited in ecosystem press put first-week retention at 1–2% | 2025–2026, NostrMag |
| Protocol development | `nostr-protocol/nips` pushed 2026-11-16; `nbd-wtf/nostr-tools` pushed 2026-11-19; `nostr-protocol/nostr` (12,152★) last pushed 2025-06-27 | GitHub API, 2026-11-24 |
| Client development | Amethyst pushed **2026-11-24** (today); Primal Android pushed 2026-11-23, web 2026-11-17, server 2026-11-18; Damus pushed 2026-09-11 | GitHub API, 2026-11-24 |
| Non-social development | Mostro v0.20.1 (Oct 22, 2026; 50 releases); White Noise rebuilt 2026, native apps in beta Sep 2026; Marmot spec superseded NIP-EE Dec 2025 | lib.rs / OpenSats / whitenoise.chat |
| Funding | OpenSats Nostr Fund: **>$10M allocated since 2023** (seeded by Jack Dorsey's #startsmall); waves 14 (Dec 2025), 15 (Feb 2026), 16 (Apr 2026), 17 (May 2026); Q4 2026 application window open Oct 1–Nov 30, 2026; HRF Bitcoin Development Fund also funds Nostr work; Mostro's grant renewed 2025 | OpenSats grant-wave posts; web3grant directory (Oct 31, 2026) |
| Community metabolism | "Weekly Nostr Recap" at its 103rd edition (Nov 2026); Nostr Compass, a Bitcoin-Optech-style technical newsletter, launched and publishing weekly | Medium / Buttondown, Nov 2026 |
| Concentration caveat | 2023 measurement: top relay hosted 73% of posts; relay lists dominated by a few defaults/bridges (see §1.8) | 2023 academic measurement |

### 2.3 Verdict: niche-but-stable

Not *alive-and-growing*: the social network peaked in 2023 and has flatlined below 100k weekly actives since, with retention so poor that advocates themselves describe decline. Not *effectively dead* either — dead protocols do not get four grant waves in six months, client pushes the same week you measure them, a relay population that holds ~1,100–1,200 answering endpoints across two years of churn, and a production exchange shipping its 50th release.

The accurate picture is a **small, stable, ideologically-committed niche whose centre of gravity is moving**: away from competing with X as a social feed, toward being a *signed-event transport and identity layer* for things that need exactly that — MLS group messaging (Marmot/White Noise), censorship-resistant exchange coordination (Mostro), wallet RPC (NWC), agent tooling (ContextVM), app distribution (Zapstore). Funding follows that shift (the 2026 waves fund infrastructure and non-social apps disproportionately). For our purposes this is the favourable quadrant: the part of Nostr that is healthiest is precisely the part we would use.

---

## Part 3 — Adapter fit against the multi-transport contract (#13929)

Assessment target: Nostr as a **carrier for our own stack** (both ends ours), under the contract's declaration discipline. Read against #13929 §§4–5 and the epic #13926's level vocabulary.

### 3.1 The declaration, filled in honestly

| Field (#13929 §4.1) | Nostr adapter declaration |
|---|---|
| `shape` | **transport** (for our-stack carriage). It becomes a *proxy* only if we ever bridge to native Nostr users — a different adapter with custody/translation declarations, not this one. |
| `addressing` | A peer is named by its carrier **npub** (secp256k1). The npub is *not* our identity: it is bound to the fleet-CA / ID identity by an attestation our stack issues and can withdraw (§3.3). Discovery of a peer's current relays follows their kind-10002 list (NIP-65), itself only trusted once the binding is verified. |
| `levels` | send: **bidirectional**, **write-capable (qualified, §3.2)** · receive: **read**, **bidirectional** · **sync-equivalent: NOT declared** (§3.4) |
| `delivery` | At-least-once (multi-relay publication duplicates; receivers dedupe by event id). No ordering guarantee; `created_at` is author-asserted and gameable. Causal metadata does **not** survive as a protocol property. Store-and-forward: delivery waits on relay retention, which is per-relay policy (hours to indefinite). Latency class: seconds. Offline behaviour: publish when any write relay is reachable; receive on next subscription sweep. |
| `size limits` | Design budget **≤ 64–100 KiB per event** for public-relay carriage (NIP-11 evidence below); up to MiB-class on relays we operate. Larger payloads move by digest reference (§3.5). |
| `identity_binding` | `cryptographic` at the payload layer (our signed envelope is verified end-to-end, carrier-independent); the npub↔identity link is an **attestation** — withdrawable at our layer, never a Nostr-native property (§3.3). |
| `capability_gating` | Sender needs a currently-valid capability per our own rules; **verification happens at receipt time** (contract §4.5), because store-and-forward means revocation may post-date sending. Relays are never part of gating. Short-lived tickets preferred; a relay-visible event confers no authority by existing. |
| `metering` | Events and bytes counted at our own relays when used, else client-side at publish/subscribe; relays we don't run cannot be metered — declared as such. |
| `custody` | n/a (transport shape). If public relays are used, the trust statement is: relays see metadata (connection IP, filters, timing, sizes, recipient npub for gift wraps) and can withhold or delay; they cannot read wrapped content or forge payloads. |

### 3.2 Levels, argued

- **read — yes.** Subscribe with filters, receive stored + live events. Durable as relay retention allows; the declaration must name that read completeness is bounded by which relays we query and what they kept.
- **bidirectional — yes, natively.** Publish and subscribe are independent operations against independent relay sets. A peer need never be online at the same time as us (store-and-forward), which several inventory carriers cannot say.
- **write-capable — yes, qualified.** The stack can originate state-changing actions on the far side: our peer acts on authenticated envelopes it receives. NWC (a wallet executes payment commands received over relays) and Mostro (an exchange daemon executes trade state machines over relays) are production existence proofs that request/response action protocols run fine on this carrier. The qualification is the contract's own: the *authority* is never Nostr's — it is our capability, verified at receipt. Declared write-capable with that gating statement, or the declaration lies.
- **sync-equivalent — no.** See §3.4.

### 3.3 Identity binding: what it can honestly mean

Our identity roots are the fleet CA (EC P-384) and the ID layer's keys; a Nostr npub is secp256k1 — a different curve, chosen by a protocol that has no CA concept. Three candidate bindings:

1. **Carrier key as the identity** (npub *is* the peer) — rejected: it imports Nostr's no-rotation/no-revocation failure model into our identity layer wholesale.
2. **Attested carrier key (chosen shape):** each stack identity publishes a binding record — our CA/ID signature over (identity, npub, validity window, relay list) — itself carried as a Nostr event and re-issued on rotation. Verification chains to *our* root, never to Nostr. Withdrawing the binding (stop honouring it; publish its successor) revokes the carrier key **at our layer**, which is the only layer where our revocation posture has ever lived (receipt-time verification, §4.5 of the contract; non-renewal in the P2P synthesis). The npub becomes what a MAC address is to a certificate: a locator with a signed claim attached.
3. **Per-purpose disposable keys under one attestation:** NWC and Mostro already work this way (per-connection keys, per-trade keys). For high-churn uses (bootstrap tickets), attest a *derivation claim* instead of individual keys. This composes with (2) and is the recommended default for anything beyond long-lived service identities.

NIP-05 is **not** a binding mechanism for us: it anchors a name in DNS/domain control and its own spec warns the name must never become the reference. It may serve as a human-readable hint, nothing more.

### 3.4 Why not sync-equivalent

The contract's test (§4.2): full data-plane sync with convergence intact — causal metadata survives, bulk transfer works, withholding/replay declared. Nostr fails each leg. Events carry an author-asserted timestamp and convention-based tags, not causal history; nothing in the relay protocol detects a withheld event. Retention varies per relay, so "the set" is whatever each relay happened to keep, and replaceable kinds *discard* prior states as a feature. NIP-77 negentropy is the strongest counter-candidate and still fails the test: it reconciles an event *set* between one client and one relay efficiently — it says nothing about causal order, cross-relay completeness, or convergence of a replicated state machine. Bulk transfer fails §3.5's size reality. **Declared: not sync-equivalent**, with negentropy noted as a useful *component* if a future design wants cheap set-difference checks against our own relays inside some other sync story.

### 3.5 Payload reality and the carriage format

NIP-11 `limitation.max_message_length` (bytes of incoming JSON, effectively the event ceiling) as observed on live relays this week: **131072 (128 KiB) is the common strfry default** across the public population; permissive relays advertise 2 MiB event / 4 MiB message (the White Noise foundation relays); the spec's own example document shows 16384. There is no protocol-level limit — it is per-relay policy, discoverable via NIP-11, and a relay *we* run can set it as we like. Honest engineering budget: **≤ ~64–100 KiB per event on public relays** (leaving headroom under the common 128 KiB cap for JSON escaping and wrap overhead); MiB-class only on our own relays.

Carriage shape for our envelope, following the ecosystem's proven patterns:

- **Kind allocation:** a dedicated kind in the regular range for cleartext-signed carriage, and gift-wrap (kind 1059) for sealed carriage. (Mostro: kind 14 rumors; NWC: 23194/23195 ephemeral-range request/response — both are kind-per-protocol conventions like ours would be.)
- **Mode A — signed cleartext:** envelope JSON as `content`, signed by the carrier key. Relay-visible: everything except what the envelope itself encrypts. Right for bootstrap tickets and public announcements — data whose authenticity matters and whose metadata is not sensitive.
- **Mode B — gift-wrapped (default):** envelope as the NIP-59 rumor, sealed to the recipient's npub. Relays learn recipient + approximate timing + size only. This is the NIP-17/Mostro-v1 pattern and the correct default for operational traffic.
- **Oversize:** chunking across events is possible but fragile (no ordering, partial retention); the ecosystem's answer is the Blossom pattern — the payload lives in a store addressed by digest, the event carries the digest + retrieval hints. For us that maps directly onto the ID layer's digest-pinned references: Nostr carries the *reference and the ticket*, another path carries the bytes. This is also the natural asymmetric composition: announce over Nostr, fetch over iroh/HTTP/sneakernet.

### 3.6 Asymmetric fit

The best in the inventory after email and sneakernet, and arguably the most natural: NIP-65 *is* per-direction path selection as a protocol feature — an identity's read relays and write relays are separate lists, and a deployment can be publish-only (write relays, no subscriptions) or listen-only. Send over Nostr while receiving over the bridge, email, or iroh requires nothing special: the envelope's reply route (contract §4.3) names the return path, and the Nostr leg simply never carries replies. A Nostr receive path also degrades gracefully as a *wake-up channel*: a small event on a public relay ("there is something for you") with the substance fetched elsewhere — the same shape as the bridge's public wake signal, on infrastructure we don't operate.

### 3.7 Weaknesses against our revocation posture, named plainly

1. **No revocation in the protocol — all of it is ours to carry.** A leaked carrier key signs convincingly forever. Our mitigations (attested bindings with validity windows, receipt-time verification, non-renewal, disposable keys) work, but they are *our* machinery riding on a carrier that contributes nothing. If our binding layer has a gap, Nostr will not catch it.
2. **Deletion does not exist.** Anything published to public relays in cleartext must be treated as published forever (NIP-09 is a request; copies persist). Consequence: Mode A is for data we would put on a public web page; everything else is Mode B or doesn't ride Nostr.
3. **Metadata exposure is structural.** Even gift-wrapped, relays see recipient npubs, timing, and sizes; subscription filters leak interests; source IPs are visible to relay operators (no built-in Tor, per White Noise's own FAQ). Correlation across relays is a real adversary capability at state level.
4. **Spam and relay economics.** Open public relays are flooded and churn fast (68% of the Jul-2024 population gone by Sep 2026). A serious deployment runs or pays for its relays — at which point part of the "someone else's infrastructure" appeal narrows to *interoperability with* a public network rather than dependence on it.
5. **Practice is more centralised than the diagram.** Discovery defaults concentrate on a handful of relays. Our adapter should pin its own relay set in the binding record rather than inherit client defaults.

### 3.8 Place in the programme

Per Drew's direction (2026-11-23): Nostr joins the #13926 inventory as an **eventual adapter** — explicitly not first-wave. The #13929 §5 order stands untouched; Nostr slots behind the sequenced families, in the same opportunistic tier as Gopher/Gemini/telnet ("support lots of dead protocols" is the stated spirit, and Part 2 shows Nostr is not even dead — merely niche). When it is built, the first honest scope is: Mode A/B carriage of bootstrap tickets and wake signals (read + bidirectional), our own relays in the binding record, write-capable enabled per-route only where receipt-time capability verification is already implemented.

---

## Sources

**Primary (read directly, 2026-11-24):** NIP-01, 05, 09, 11, 17, 26, 40, 42, 44, 47, 59, 65, 70 — `nostr-protocol/nips` (pushed 2026-11-16). GitHub API repository records for `nostr-protocol/nips`, `nostr-protocol/nostr`, `damus-io/damus`, `vitorpamplona/amethyst`, `nbd-wtf/nostr-tools`, `PrimalHQ/*` (pushed-at dates as cited). Live NIP-11 documents via the Yadha relay directory (strfry defaults 131072; IPF/White Noise relays 2 MiB event / 4 MiB message; nostr.com relay 131072, 60-day retention).

**Secondary:** nostr.co.uk, "Is the Nostr Relay Network Shrinking?" (NIP-66 monitor analysis, Sep 2026) and Q1 2026 ecosystem roundup · NTNU IFAC-2026 preprint, "Systematic Analysis and Empirical Study of the Decentralized Social Network: The Nostr Case" (Nov 2025 telemetry) · Punk Science tech brief, "Nostr as a Foundation for Social Media" (WAU ceiling, Manifold resolution, Svetski quote, 2023 concentration measurement) · OpenSats Nostr grant waves 14 (2025-12-21), 15 (2026-02-17), 16 (2026-04-08), 17 (May 2026) and Mostro project page (grant renewed 2025) · web3grant Nostr grants guide (fund total >$10M, Q4 2026 window) · Mostro documentation via lib.rs (v0.20.1, protocol v2, NIP-69) and OpenSats project page · White Noise / Marmot: whitenoise.chat year-in-review (2024–2025), jgmontoya.com "Private Messaging on Nostr: From NIP-04 to Marmot" (Jul 2026), crays.org NIP-EE archive (supersession record) · Weekly Nostr Recap editions 81–103 (Jun–Nov 2026) for dashboard slices (nostrhub, stats.andotherstuff, npub.world) — used as samples, with their inflation problems noted in §2.1 · Glukhov.org Nostr overview (Oct 2025) — small-slice figures flagged as method-unclear where used.
