# Design spec: hosted provisioning + SSO→ID binding

- **Issue:** developing-today/code#2258 (implementation spec for the #2045 investigation)
- **Status:** spec only. No code in this document's change. Implementation is filed
  separately (see §9); nothing permanent is built on an unreviewed investigation
  (program rule, epic #2056).
- **Sources:** #2045 investigation result (queue task 3354, posted on #2045,
  2026-10-29) and #2044 Workers-fit investigation
  (`2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit`). Where this spec
  and #2045 disagree, #2045's recorded decisions win and this spec is the defect.
- **Open parameters** are listed in §8 and routed to #2046. They are **not**
  defaulted here.

## 0. Shape in one paragraph

The hosted offer is #2045's Shape B: one SQLite-backed Durable Object (DO) per
user, lazy-provisioned, hibernating, costing nothing while idle; blobs in R2,
content-addressed. Google SSO authenticates the *account*; the Iroh root key
*is* the identity. The two are connected only by the binding ceremony in §3 —
sign-in alone never binds, claims, or rebinds anything.

## 1. Records

### 1.1 Account record (service accounts registry)

One row per Google identity. The registry is service-operated storage, separate
from every user's DO; it holds no user content.

| Field | Rule |
|---|---|
| `sub` | Google's stable subject identifier. **Primary key.** Never the email address (emails change, are recycled, and are shared in ways `sub` is not). |
| `account_id` | Service-minted opaque ID, assigned at first sign-in, immutable. |
| `email_contact` | Optional contact metadata from the Google profile, used only for notifications (notably rebind notices, §4). Never a key, never an identity claim. |
| `created_at` | First sign-in time. |
| `binding_id` | Null until first binding (§3); then the stable binding record ID (§1.2). |
| `tier` | Tier identifier; the numbers behind each tier are a #2046 parameter (§8). |
| `meter_rollup` | Latest daily metering rollup from the user's DO (§5.3). |

An account with no binding is an *empty hosted account*: it can sign in and
start a ceremony, and can claim, read, and write nothing.

### 1.2 Binding record

The binding record is the only artefact that connects a Google `sub` to an
Iroh ID. It has a stable identity across rebinds, so the user's DO — named
from `binding_id` (§5.1) — never moves when the key changes.

```json
{
  "binding_id": "<opaque, minted at first binding, immutable>",
  "version": 1,
  "sub": "<google sub>",
  "account_id": "<account record id>",
  "iroh_id": "<Iroh root public key, current>",
  "bound_at": "<RFC 3339>",
  "challenge_id": "<the ceremony challenge this binding was proven with>",
  "challenge_transcript": {
    "challenge_id": "<same challenge_id>",
    "sub": "<google sub>",
    "iroh_id": "<Iroh root public key>",
    "service_id": "<service identifier>",
    "nonce": "<the challenge's 32 random bytes, base64>",
    "issued_at": "<RFC 3339>",
    "expires_at": "<RFC 3339>"
  },
  "key_signature": "<Iroh root key signature over the canonical ceremony transcript (§3.2)>",
  "service_attestation": "<service key signature over SHA-256 of the canonical record without this field>",
  "status": "active | rebind_pending | superseded",
  "history": [
    { "iroh_id": "<previous key>", "superseded_at": "<RFC 3339>",
      "via": "old-key-signature | delayed-rebind",
      "rebind_record_id": "<§4.4 record, when via delayed-rebind>" }
  ]
}
```

Rules:

- Exactly one `active` binding per `sub`, and one `sub` per `iroh_id`, at any
  time. A ceremony that would violate either is a conflict (§3.4), never an
  overwrite.
- A binding in `rebind_pending` (§4.2) still **reserves** the account: it
  counts as the account's binding for every conflict check in §3.2 and
  §3.4. No fresh ceremony may complete while a rebind is pending — a
  replacement key reaches the binding only through §4 (old-key
  cancellation, early approval, or the delay elapsing), never by starting
  a new initial binding that overwrites `account.binding_id`.
- The record is valid only with **both** signatures: the key's (proof of
  possession at ceremony time) and the service's (attestation that the
  ceremony ran against an authenticated session for `sub`).
- The complete `challenge_transcript` is persisted **in the record**, not
  only in the service's challenge store: the signed message in §3.2 is
  reconstructed from `challenge_id`, `sub`, `iroh_id`, `nonce`, and
  `expires_at`, so a record stripped of its transcript cannot be verified
  offline. The transcript travels with the record everywhere the record
  goes — in particular in exports (§6.1) — so an importer (§6.2) verifies
  `key_signature` with no access to the hosted service.
- `history` is append-only. A superseded key's entries are never deleted;
  they are how an importer (§6) verifies the chain.

### 1.3 Device linkage

Devices do **not** bind individually and get no binding records. The binding
names the root/person Iroh ID only; devices join through the platform's
existing device-grant mechanism in the `groups` layer, exactly as in the
self-hosted model. A device grant references the bound `iroh_id`; revoking a
device is a groups-layer operation and never touches the binding record.

## 2. Provisioning hooks (summary; full protocol in §5)

- First sign-in creates the account record (§1.1) and nothing else.
- First completed binding mints `binding_id`, names the user's DO, and the
  DO's schema materializes on its first request (§5.1–§5.2).

## 3. Binding ceremony

### 3.1 Preconditions

- An authenticated Google session for the account (OIDC `sub` verified from
  the ID token by the service; the client never asserts a `sub`).
- The user controls an Iroh root key: generated/held in the web client
  (in-browser variant, §3.3a) or on their own node (§3.3b).

### 3.2 Flow (both variants)

1. **Request challenge.** Client sends `{iroh_id}` over the authenticated
   session. Service mints a challenge:
   `{challenge_id, sub, iroh_id, service_id, nonce (32 random bytes),
   issued_at, expires_at}` with a 5-minute lifetime, single-use.
2. **Sign.** The Iroh root key signs the canonical transcript:
   `id-binding/v1 || challenge_id || sub || iroh_id || nonce || expires_at`
   (length-prefixed fields, UTF-8). This binds the proof to this account,
   this key, and this challenge — a signature replayed against another
   account or challenge does not verify.
3. **Submit proof.** Client returns `{challenge_id, key_signature}`.
4. **Verify.** Service checks, in order: challenge exists and is unexpired;
   challenge is unconsumed; session `sub` == challenge `sub`; signature
   verifies against `iroh_id`; no binding conflicts (§1.2 rules) — an
   existing binding in `active` **or** `rebind_pending` status both
   conflict, and the pending case is routed to §4, never completed here.
   **The challenge is consumed on first presentation, success or failure** —
   a failed attempt can never be retried against the same challenge.
5. **Attest and store.** Service writes the binding record (§1.2) with its
   attestation signature, sets `account.binding_id`, provisions the DO name
   (§5.1), and returns the completed record to the client.

### 3.3 Variants

**(a) In-browser.** The web client generates or unlocks the root key locally
(WebCrypto / WASM signer) and performs steps 1–5 over the same session. The
key never leaves the device in any form; the service sees only the public
`iroh_id` and signatures. If the browser profile is the key's only home, the
client must surface the export-your-key step as part of the ceremony's final
screen — a binding whose only key copy dies with a browser profile is a
support incident scheduled in advance.

**(b) From own node.** The node holds the key; the browser session is only
the authenticated channel. The browser displays a short-lived pairing code
tied to the pending ceremony; the user enters it on their node
(`id bind` flow). The node exchanges the code for the challenge (step 1 runs
node-side), signs locally (step 2 — the key never leaves the node), and
submits the proof (step 3) against the same `challenge_id`. Pairing codes are
single-use, expire with the challenge, and are rate-limited per account
(§3.4). Steps 4–5 are identical.

### 3.4 Failure and retry semantics

| Failure | State change | Retry |
|---|---|---|
| Challenge expired | None; challenge dead | Request a fresh challenge |
| Bad signature | None; challenge consumed | Fresh challenge; repeated failures rate-limit challenge issuance per account |
| Challenge replay (already consumed) | None; rejected and logged | Fresh challenge |
| `iroh_id` already bound to another `sub` | None; conflict error naming the rebind path (§4) | Not retryable as a fresh binding |
| `sub` already has an active binding (different key) | None; conflict error naming §4 | Rebind protocol, not the ceremony |
| `sub` has a binding in `rebind_pending` | None; conflict error naming §4 | §4 only: old-key cancel/approve, or wait out the delay — a fresh ceremony never bypasses it |
| Two ceremonies race for one `sub` | First completed wins | Loser receives the conflict error |

No failure path creates a partial binding, a DO, or any claim on the key.

## 4. Delayed-rebind protocol

Rebind = replacing the `iroh_id` in an existing binding record. It exists
because losing the Iroh key must not lock the user out forever, and Google
sign-in alone must **never** silently rebind (that would convert any Google
account takeover into an ID takeover — #2045).

### 4.1 Two paths

- **Fast path (old key available):** the old root key signs a rebind
  authorization over `{binding_id, new_iroh_id, challenge_id}` from a fresh
  §3 challenge, and the new key countersigns proof of possession. Effect is
  immediate on verification.
- **Delayed path (old key lost):** as specified in §4.2–§4.4. This is the
  only path that does not require the old key, and its delay is the security
  property.

### 4.2 Delayed path: request

1. Authenticated Google session for the binding's `sub`, plus a fresh §3
   challenge proving possession of the **new** key.
2. Service creates a rebind request:
   `{rebind_id, binding_id, sub, old_iroh_id, new_iroh_id, requested_at,
   effective_at = requested_at + waiting_period, status: pending}` and sets
   the binding's `status` to `rebind_pending`.
3. During the wait: the **old key keeps full control**; the new key has
   none; the account's data and DO are untouched. The pending binding
   continues to reserve the account (§1.2): any fresh binding ceremony
   for this `sub` is rejected as a conflict (§3.4) for the whole wait, so
   the delay cannot be bypassed by re-running §3 with another key.

The waiting period's length is a #2046 parameter (§8). #2045 bounds it only
as *days, not minutes*; this spec deliberately states no number.

### 4.3 Notification and cancellation

On request creation, the service notifies every recorded contact point:
`email_contact` (§1.1), a signed notice object delivered to the old key's
known platform endpoints/worlds, and a banner on any old-key session. The
notice names the new `iroh_id` (fingerprint form), `effective_at`, and the
cancellation method.

The old key can, at any time before `effective_at`:

- **Cancel:** sign `id-rebind-cancel/v1 || rebind_id` with the old root key.
  Cancellation is final for that request; a later request restarts the full
  waiting period.
- **Approve early:** sign the §4.1 fast-path authorization, converting the
  pending request to an immediate rebind.

An ignored notice is consent by design: if the old key is truly lost, nobody
can cancel, and the rebind proceeds at `effective_at`.

### 4.4 Publication

At effect (either path), the service:

1. Appends the old key to the binding's `history` (§1.2) with `via` set.
2. Sets `iroh_id` to the new key, `status` back to `active`.
3. Publishes a **rebind record** — `{rebind_id, binding_id, old_iroh_id,
   new_iroh_id, effective_at, via}` signed by the service and countersigned
   by the new key — to the same surfaces the platform uses for signed graph
   objects, so peers and importers learn the new key from a verifiable
   artefact, not from the service's say-so.

Losing **Google** access is out of scope by design: it is Google's recovery
problem, the binding survives it, and regained Google access re-enters the
same account (§1.1).

## 5. Provisioning and metering

### 5.1 Lazy provisioning

- Account: first sign-in, one registry row (§2). An account that never
  returns costs that row and nothing else.
- DO: named `idFromName(binding_id)` at first binding. The object and its
  SQLite storage materialize on first use; there is no sign-up-time fleet,
  no per-user container, and no always-on anything (Shapes A–C idle at $0;
  Shape D containers are rejected as a tier — #2045).
- Blobs: content above the DO SQLite row cap (2 MB, #2044) goes to R2,
  content-addressed by hash; the DO keeps per-user reference rows
  `{blob_hash, size, first_seen}`. Identical plaintext blobs dedupe across
  users; client-encrypted blobs (§7) dedupe only per user.

### 5.2 Schema bootstrap

The DO's first request runs versioned migrations before serving: a
`schema_meta` table records the applied version; migrations are idempotent
and run under the DO's serialized execution, so concurrent first requests
cannot interleave a bootstrap. A migration failure leaves the DO unserved
(error to the caller, alarm to the service log) rather than half-created —
the next request retries the bootstrap from the recorded version.

### 5.3 Metering

Each DO meters its own usage in its storage, so metering survives
hibernation and needs no external pipeline per request:

- Two kinds of metric, kept distinct:
  - **Current-usage gauges** — SQLite bytes and R2 referenced bytes.
    These measure what is stored *now* and **decrease** when the user
    deletes data (a blob's referenced bytes drop when its last reference
    from this DO is removed). Storage ceilings (§5.4) read only these.
  - **Monotonic consumption counters** — request count, WebSocket
    session-seconds, R2 egress bytes, ephemeral-job seconds (§5.5).
    These measure cumulative consumption over the billing period and
    never decrease within it.
- Both kinds are snapshotted daily into the account record's
  `meter_rollup` (§1.1), each labelled with its kind. Billing/ceiling
  decisions read the rollup plus the live DO values, never a third
  source. Conflating the kinds is a defect: monotonic storage accounting
  would make the §5.4 recovery path (free space, resume writes)
  impossible, since deleted data would never return allowance.
- Tenant ceilings from the platform (per-DO storage and request-rate caps,
  #2044) are a blast-radius property, not a metering input: one runaway
  user degrades their own DO, not the fleet. Fleet-level abuse controls
  remain #2032's layer.

### 5.4 Budget ceilings and graceful degradation

Tier ceilings (storage bytes, monthly requests) are #2046 parameters (§8).
Behaviour at a ceiling is specified here and is not tier-dependent:

- **Storage cap reached:** the store goes **read-only** and every session
  surfaces the export prompt (§6). Writes are refused with a named
  `storage_cap` error. Data is never deleted, truncated, or silently
  dropped to fit.
- **Request ceiling reached:** requests are refused (`429`, `Retry-After`)
  with the same export prompt on the client surfaces. Reads needed to
  *perform an export* are exempt from the ceiling — leaving must always be
  possible.
- **Recovery:** raising the tier (or freeing space by export + explicit
  user-confirmed delete — which lowers the current-usage gauges in §5.3)
  returns the store to read-write automatically at the next request; no
  support action is part of the mechanism.

### 5.5 Ephemeral containers

Heavy jobs (bulk import/export, heavy world compute) may use containers
only under #2045's rule: every spawn names an owner, a max runtime, and an
auto-stop; runtime is metered to the job's owner (§5.3). A container found
running without all three is a bug to file, not a pattern to copy.

## 6. Export and import

### 6.1 Export format

An export is a set of signed objects plus a manifest, producible on demand
(including from a read-only, at-cap store, §5.4):

1. **Objects:** every signed graph object in the user's DO, in canonical
   (hash) order — self-authenticating by construction.
2. **Blobs:** every R2 blob referenced by those objects, by hash.
3. **Records:** the binding record (§1.2) including full `history` and
   the complete `challenge_transcript` (§1.2), any rebind records (§4.4),
   and the account metadata needed to re-establish the binding claim
   (`sub` assertion as attested by the service).
4. **Manifest:** `{export_id, binding_id, iroh_id, created_at,
   object_count, blob_count, sha256 of the concatenated object hashes}`,
   signed by the user's Iroh root key when available.

If the user's key is unavailable (e.g. export during a lost-key rebind
wait), the service produces a **service-signed manifest marked
`unkeyed_export`**. The data is identical; the trust differs, and §6.2
treats it differently.

### 6.2 Import verification (self-hosted node)

The importing node verifies before ingesting anything:

1. Manifest signature: key-signed manifest must verify against the
   `iroh_id` it names; an `unkeyed_export` manifest is accepted only as an
   explicitly confirmed untrusted-source import by the node's operator.
2. Binding chain: the binding record's key signature and service
   attestation verify (§1.2) — the §3.2 transcript is reconstructed from
   the exported record's own `challenge_transcript` (`nonce` and
   `expires_at` included), never fetched from the hosted service — and
   every `history` step is backed by its rebind record (§4.4) or an
   old-key fast-path signature.
3. Integrity: every object's hash and signature verify; the manifest's
   counts and concatenated-hash digest match the received set exactly.
   **Blob contents are verified too, before anything is ingested:**
   every received blob is hashed, and its hash must equal the content
   hash referenced by the signed object(s) that cite it. Object
   signatures and the object-hash digest do not cover blob bytes (graph
   ingest treats `document.put` content as a separate blob referenced by
   hash, not an event body), so without this step a corrupted or
   substituted blob would pass every other check. A blob whose hash
   mismatches, or that no verified object references, fails the whole
   import. A partial or tampered export imports nothing.
4. Ingest then follows the platform's normal rules (#2030: validity at
   ingest, counting at view) — import is not a privileged write path, and
   hosted-origin objects are indistinguishable from self-hosted ones at
   the object layer.

## 7. Client-side-encryption option

Offered at binding time (and changeable later only by re-encrypting via
export/import), with the trade stated in the offer's own words per #2045:
hosted storage is operator-readable at rest unless this option is on.

- **What is encrypted:** object payloads and blob contents, client-side,
  before upload, under keys derived from and held by the user's devices.
  Keys never leave the devices; the service cannot decrypt.
- **What stays plaintext:** the metadata metering and sync require —
  object/blob hashes and sizes, timestamps, graph shape (references),
  and the binding/account records (§1). The DO stores opaque payloads.
- **What degrades:** hosted server-side search and indexing, content
  previews/thumbnails, any Shape-C world feature that must read content
  server-side, and web-only access from a device that does not hold the
  keys. Features degrade to what the metadata allows; they do not fail.
- **Dedupe interaction (accepted, #2045):** encrypted blobs dedupe only
  per user; the cross-user R2 dedupe of §5.1 does not apply to them.
- **Default:** not decided here. Whether encryption is default-on is a
  #2046 / Drew decision (§8); #2045's recommendation on record is to offer
  it at binding time with the feature trade explained.

## 8. Open parameters — routed to #2046, not defaulted here

| Parameter | Notes from #2045 |
|---|---|
| Free-tier storage / request ceilings | Numbers only; behaviour at cap is §5.4 |
| Paid-tier numbers and prices | #2032 budget pattern applies |
| Rebind waiting period | Days, not minutes — exact length is the call |
| Client-side-encryption default (on/off at binding) | Genuine values call for Drew; recommendation in §7 |
| Shape-C cost attribution when a world/app owner lapses | Needed before Shape C ships |
| Pairing-code / challenge rate limits (§3.4) | Mechanism specified; thresholds are operational tuning |

## 9. Implementation routing

Spec only — implementation is filed as its own issue and linked from
#2258, sliced as: (1) account/binding records + ceremony (§1, §3);
(2) delayed rebind (§4); (3) lazy provisioning + metering + ceilings (§5);
(4) export/import (§6); with §7 as a flag through all four. Slices are
sequenced after the #2046 synthesis accepts §8's parameters.
