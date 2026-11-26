# #10706 addendum: carrying a world announcement in pkarr

Status: decided, with a reference. Implementation follows in slices
(below). Parent design:
[#6287 discovery, registry and federation v1](../2026-11-16_design_id6287_discovery_registry_federation_v1.md)
§3 (the announcement), §4 (publish-to-pkarr) and §6 (resolver order).

## Why this exists

#10706 asks for "publish the signed announcement to pkarr, republish on
change, heartbeat at half the effective lifetime". The packet
(`id-sync/src/announcement.rs`, #10705) is done, but three contracts
were never written down, so two implementers would have shipped two
incompatible formats:

1. how the announcement is placed inside a pkarr signed DNS packet;
2. which sinks a self-hosted operator publishes to, and what counts as
   published;
3. what drives the change and heartbeat republishes, and how `seq`
   relates to the pkarr timestamp.

This note fixes all three. `fixtures/pkarr_reference.py` is the
executable form, and `fixtures/vectors.json` holds the known-answer
vectors the Rust code must reproduce.

## 1. Encoding

The pkarr key is the world key (`announcement.world_id`).

- **Record:** exactly one `TXT` record, class `IN`, TTL 300, named
  `_ida.<z32 world key>`. The name is written fully qualified, the form
  pkarr clients normalise to; a resolver also accepts the relative name
  `_ida`.
- **rdata:** the bytes of `SignedAnnouncement::to_bytes()` (CBOR),
  unencoded, split into DNS character-strings of at most 255 bytes, in
  order. A resolver joins every character-string of the record and
  decodes the result. Raw bytes, not base64: base64 would cost a third
  of the budget, and TXT character-strings are length-prefixed binary.
- **Packet:** a DNS response header (id 0, flags `0x8400`, one answer,
  no question, authority or additional records), then the record. Names
  are never compressed.
- **Relay payload and signature:** the standard pkarr framing,
  `signature(64) ‖ timestamp_us_be(8) ‖ dns_packet`, where the signature
  is the world key's ed25519 signature over the BEP44 bencoding
  `3:seqi<timestamp_us>e1:v<len>:<dns_packet>` (no salt).

**Budget.** A pkarr DNS packet is at most 1000 bytes. With a 12-byte
header, a 59-byte name, 10 bytes of type, class, TTL and length, and one
length byte per 255-byte chunk, a signed announcement of `P` bytes
needs `81 + P + ceil(P / 255)` bytes, so `P ≤ 915`.
`announcement::MAX_PACKET_BYTES` drops from 1000 to **915**, so every
announcement that `to_bytes()` accepts can be published, and an
over-budget announcement is still refused at encode time. The pinned
test announcements are about 500 bytes, so nothing real is lost.

**Resolver checks, in order** (all are refusals, never repairs):

1. the pkarr signature verifies under the queried key;
2. the packet carries exactly one announcement record (none and two
   both refuse);
3. the record decodes as a `SignedAnnouncement`;
4. `announcement.world_id` equals the queried key, so another world's
   valid announcement republished under this key is refused;
5. the announcement's own signature verifies (`SignedAnnouncement::verify`);
6. `announcement.seq` equals the packet timestamp;
7. the announcement is not expired (`verify_at(now)`).

Other records in the packet are ignored, so a later addition can share
it.

## 2. Sequence and timing

- **seq is the pkarr timestamp.** Each publish picks
  `timestamp_us = max(now_us, previous_timestamp_us + 1)` and signs an
  announcement with `seq = timestamp_us` and `issued_at = now`. One
  number then orders both the BEP44 item and §3's per-world sequence,
  and `accepts_after` keeps working unchanged. The `+ 1` keeps it
  strictly increasing through a clock step backwards.
- **Every publish re-signs.** A heartbeat is not a replay of the old
  bytes: it is a fresh announcement with the same fields and a new
  `issued_at`, `expires_at` and `seq`.
- **Lifetime:** default 3 hours, operator-configurable within the
  existing `1..=7 days` rule. A world that stops publishing is void
  within one lifetime.
- **Heartbeat:** `min(lifetime, 2 h) / 2`, where 2 hours is the
  Mainline DHT's retention of a BEP44 item. With the default that is
  hourly, so the item never lapses from the DHT while the world is up,
  and two missed heartbeats still leave it valid.
- **Change:** any change to a published field (name, description,
  subject, occupancy, tags, visibility) publishes at once and restarts
  the heartbeat timer.

## 3. Sinks, configuration and custody

- **Sinks.** An operator configures a list of pkarr relay base URLs. A
  publish is an HTTP `PUT <base>/<z32 world key>` with the relay
  payload as the body; a relay fetch is `GET <base>/<z32>`. Our relay's
  base is `https://pkarr.developing-today.workers.dev/pkarr`; it is the
  default list on our hosted instance and the suggested default for a
  self-host, and an empty list is valid. With the `dht` feature on, the
  same signed item is also put to the Mainline DHT directly.
- **Success.** A publish succeeds when at least one sink accepts it. A
  sink that refuses or is unreachable is retried at the next heartbeat;
  a refusal because a newer item is already stored means another
  publisher holds the key, which is reported, not retried.
- **Visibility.** `unlisted` and `listed` both publish to pkarr; only
  `listed` is also submitted to registries (§6). A `private` world has
  no announcement, so it runs no publisher.
- **Custody.** The publisher signs with the world key, which only the
  world's operator holds (§4). The world key must not be the iroh
  endpoint key of the node it announces: a pkarr key holds one packet,
  and iroh's own address publishing would overwrite the announcement.
  The publisher refuses a `Subject::Endpoint` whose `node_id` is the
  world key.

## 4. Where it lands

- `id-sync/src/announcement_pkarr.rs`: the encoder and resolver checks
  of section 1, and a pure `Publisher` state machine for section 2
  (`on_change`, `on_tick(now)` returning the next signed payload or
  nothing, and the next deadline). No I/O; tests reproduce
  `vectors.json`.
- A relay client (`PUT`/`GET` against a configured base) and the
  optional DHT put behind the `dht` feature.
- The world runtime drives the `Publisher` from its existing timer and
  from field changes; operator configuration holds the relay list and
  the lifetime.
- The resolver order of §6 (#10708) uses the section 1 checks for its
  pkarr step.

## Checks

`python3 -I fixtures/pkarr_reference.py check` reproduces the pinned
`WORLD_BODY_HEX` and `WORLD_SIGNATURE_HEX` from `announcement.rs` (so
its CBOR and ed25519 match the Rust ones), rebuilds `vectors.json`, and
runs the budget, decode, refusal and timing checks. The relay payload
in `vectors.json` was also verified by `pkgs/pkarr-worker/src/pkarr.ts`
(`verifyPacket`, `parseDnsMessage`), the code our relay runs.
