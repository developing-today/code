# Plan: `fleet cert dns` — CAA and DANE DNS plan contract

Issue: developing-today/code#13724 (§6.1 verb group, child of #9157,
program #9155). Slice 1 of #13724: the settled contract. No code, no
live DNS — this document is the deliverable the implementation slice
builds against.

## Sources

- Governing spec: `doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/sources/fleet_architecture_spec_v0_3.md`
  — §6.1 (command tree: `dns plan | render | verify`), §6.3 (flag
  grammar), §4.4 (DNS/publication manifest), §24.8 (CAA, CT, DCV and
  public ACME).
- Program gate map: #9155 comment 7113208709 — live DNS/publication
  changes are behind the production gate; design work and synthetic
  proof are not.
- Landed foundation: PR #13720 (fleet-cert crate, bundle format
  `fleet-cert-manifest/v1`, core verbs). The publication half already
  implements this contract's shape for artifacts: schema
  `fleet.pki.publish/v1`, a `DnsRecord { type, name, ttl, value,
  action }` record type, and read-only `plan`/`verify` with `apply`
  explicitly deferred (`pkgs/fleet/crates/fleet-cert/src/publish.rs`).
  The `dns` verb group reuses that record shape and that posture; it
  does not invent a parallel one.

## 1. The corrected CAA premise

The issue body says "CAA records constraining issuance to the fleet CA
set". That is wrong, and this contract corrects it (spec §24.8):

- **CAA governs publicly trusted CA issuance** (WebPKI / public ACME —
  e.g. Let's Encrypt, ZeroSSL). A public CA checks CAA before issuing;
  the records authorize or forbid *public* issuers by domain name.
- **CAA does not constrain the private Fleet CA.** A private CA
  installed as a trust anchor is not a CAA checker and is not bound by
  CAA; a CAA record naming the Fleet CA authorizes nothing and forbids
  nothing. The planner therefore never emits a CAA record whose value
  names the Fleet CA, and a request to do so is a contract error with
  an explanation, not a rendered record.
- A public DNS name under the private CA does not become
  browser-trusted because CAA (or CT) records exist for it (§24.8).
- What CAA planning is *for* in this program: the fleet's public names
  that also get public ACME certificates (the §24.8 public-ACME
  subsystem) get CAA records that pin issuance to the intended public
  issuers — and nothing else.

## 2. Command contract

All three verbs are **offline**: they read local files (bundles,
policy input, an optional recorded current-state snapshot) and write
local files. None of them opens a network socket, queries a resolver,
or talks to a DNS provider. This is a property of the contract, not a
flag: the verbs accept no provider, endpoint, or credential arguments.

```text
fleet cert dns plan    --zone <apex> [--bundle <dir>]... [--policy <file>]
                       [--current <snapshot-file>] [--ttl <seconds>]
                       [--out <dir>] [--report json|yaml|text]
fleet cert dns render  --change-set <file> [--format zone|json|yaml]
                       [--out <dir>] [--report ...]
fleet cert dns verify  --change-set <file> [--bundle <dir>]...
                       [--report ...]
```

- `plan` computes the desired CAA/TLSA record set from the inputs and
  emits a versioned **change-set** (§3). With `--current` (a recorded
  snapshot file in the same record shape — data an operator captured
  earlier, never a live query), the change-set carries per-record
  actions (`create` / `no-op` / `delete`); without it, every record is
  `conditional-create` per §4.4. If the desired set already equals the
  supplied current set, `plan` reports **no changes** — it never
  fabricates changes (§4.4).
- `render` turns an existing change-set into its presentations: the
  BIND zone fragment (`dns.zone`, RFC 1035 style, as in §4.4) and the
  canonical JSON/YAML. Rendering is pure: same change-set in, same
  bytes out; it recomputes nothing.
- `verify` checks a change-set **offline against the bundles**: TLSA
  associations recompute from the referenced certificates, CAA values
  parse and name only public issuers, owner names are well-formed,
  TTLs are in range, and every record the change-set claims is
  DNSSEC-gated is marked as such (§4). It returns one named check per
  item (the `publish.rs` verify-report shape) and a boolean `valid`.
  `verify` never compares against live DNS — see §5.
- Common grammar follows §6.3: `--dry-run` is redundant (every verb is
  a dry run by construction) and is accepted as a no-op for grammar
  parity; `--report json` emits the change-set or verify report
  verbatim; exit codes follow the existing cert-verb contract.

There is **no `dns apply` in this contract**. §6.1's tree lists only
`plan | render | verify`. A future apply verb is a separate,
production-gated design (§6), not an extension of this slice.

## 3. The change-set (versioned output)

Schema id: **`fleet.pki.dns/v1`** (sibling of `fleet.pki.publish/v1`;
a change-set may be embedded as the `records` of a publish plan, whose
record fields it shares).

```yaml
schema: fleet.pki.dns/v1
zone: developing.today            # apex the records belong to
generated_at: <RFC 3339>
source_bundles: [<bundle ids the TLSA associations derive from>]
records:
  - type: CAA | TLSA              # v1 scope; A/AAAA/CNAME stay with publish (§4.4)
    name: <FQDN, trailing dot>    # owner name
    ttl: 3600
    value: <record rdata, presentation format>
    action: create | conditional-create | no-op | delete
    requires: [dnssec]            # optional gates, see §4
    note: <why this record exists>
prerequisites: [<strings, §4.4 style>]
```

Rules:

- Record fields are exactly the landed `DnsRecord` shape
  (`type, name, ttl, value, action`) plus the optional `requires` and
  `note`. Consumers that read `fleet.pki.publish/v1` records can read
  these unchanged.
- Default TTL is 3600 (§4.4 example); `--ttl` overrides per plan run,
  never per record in v1.
- Values are presentation-format rdata as they will appear in the zone
  fragment, so `render` is a formatting step, not a second planner.
- The change-set is the unit of review and of `verify`. Zone text is
  derived from it, never the reverse.

## 4. TLSA / DANE rules

- **Owner name** (RFC 6698): `_<port>._<proto>.<host>`, e.g.
  `_443._tcp.pki.developing.today.` Inputs name the endpoint
  (host, port, protocol) and the certificate to associate: a leaf
  bundle, or an issuer bundle for TA pinning. The planner derives the
  owner name; callers never hand-write it.
- **Certificate association choices** (usage / selector / matching):
  - Default for a leaf endpoint: **3 1 1** (DANE-EE, SPKI,
    SHA-256) — pins the leaf's public key, so certificate reissuance
    with the same key does not churn DNS.
  - Issuer pinning: **2 1 1** (DANE-TA, SPKI, SHA-256) against the
    issuing CA certificate from the issuer bundle.
  - Usages 0/1 (PKIX) are not emitted in v1: they layer WebPKI path
    validation over DANE and are a deliberate later decision, recorded
    here as deferred, not forgotten.
  - Matching type 2 (SHA-512) and selector 0 (full certificate) are
    accepted as explicit inputs, never defaults.
- **DNSSEC is a hard prerequisite for DANE to mean anything.** A TLSA
  record in an unsigned zone is a hint any on-path attacker can
  rewrite. Therefore: every TLSA record in a change-set carries
  `requires: [dnssec]`; `plan` lists "zone is DNSSEC-signed and the
  signing is maintained" in `prerequisites`; and `verify` reports the
  record as *proposed*, never *proven*, until a post-gate live check
  validates the chain from a DNSSEC-validating resolver (§5). The
  planner does not plan DNSSEC signing itself (no DS/DNSKEY records
  in v1) — it states the requirement and stops.
- **Endpoint assumption**: TLSA pins the key of the service that
  actually terminates TLS at that name. Planning a TLSA record for a
  name whose TLS is terminated elsewhere (a CDN, a tunnel ingress) is
  an input error the operator owns; the plan records the assumed
  endpoint in the record `note` so review can catch a mismatch.

## 5. Offline proposal vs live proof

Three distinct states, and the tooling never blurs them:

1. **Proposed** — a change-set exists and `verify` passes offline.
   This is the strongest claim this slice's verbs can make.
2. **Published** — someone with production-gate approval applied the
   change-set to live DNS. Outside this contract (§6).
3. **Proven** — a live, DNSSEC-validating read-back matches the
   change-set. Also outside this contract; a later slice designs the
   live-proof verb under the same gate as apply.

Any output wording that implies state 2 or 3 from state-1 evidence is
a contract violation (the §24.8 rule: generated records are never "a
proof of live deployment").

## 6. The production gate (#9155)

Per the #9155 gate map, **not** behind the gate: this contract, the
implementation of `plan | render | verify`, and synthetic-fixture
proof without limit. **Behind the gate** (stop, report, wait for
Drew): any live DNS query-for-proof or write, any provider credential
use, any production trust-store change. The implementation slice
inherits this boundary unchanged: its verbs take no arguments that
could cross it.

## 7. Synthetic fixture oracle

The implementation slice proves the contract against fixtures only:

- **Fixture set**: the fleet-cert crate's synthetic hierarchy fixtures
  (test-only roots/issuers/leaves) plus one public-ACME scenario input
  naming a public issuer domain. No real zone, no real CA names in
  golden files beyond the public-issuer domain the scenario supplies.
- **Golden change-sets**: `plan` output for the fixture inputs equals
  the checked-in golden change-set byte-for-byte (modulo
  `generated_at`).
- **Digest recomputation**: every TLSA association in the golden set
  is recomputed from the fixture certificates by an independent path
  in the test (not by calling the planner's own digest code).
- **The CAA correction case**: a fixture whose only CA is the private
  Fleet CA produces a change-set with **zero CAA records** and a
  prerequisite note explaining why (§1). This is the oracle that pins
  the corrected premise.
- **No-network / no-write checks**: the fixture run executes with no
  DNS configuration and asserts the only files written are under the
  test's output directory; the verbs' argument surface (no provider /
  endpoint / credential flags) is asserted from the CLI help schema.
- **No-change case**: `plan --current` fed a snapshot equal to the
  desired set reports no changes.

## 8. Alternatives considered

- **Fold CAA/TLSA into `publish plan` only.** The publish plan already
  carries A/AAAA/CNAME records for the distribution host. Rejected as
  the *sole* surface: §6.1 gives `dns` its own verb group, DANE
  applies to service endpoints beyond the publish host, and the
  change-set needs its own review/verify lifecycle. The two compose
  instead (shared record shape, embeddable change-set).
- **Plan against live DNS reads.** Rejected: it crosses the #9155
  gate and makes plans non-reproducible. The `--current` snapshot
  file gives diffing without a query.
- **Emit PKIX usages 0/1 by default.** Rejected for v1 (§4): DANE-EE /
  DANE-TA are the associations a private-fleet endpoint can actually
  honor.

## Follow-up

Scoped implementation slice (filed as a comment on #13724 with this
plan): implement `fleet cert dns plan | render | verify` in
`pkgs/fleet/crates/fleet-cert` against this contract — new `dns`
module reusing the `DnsRecord` shape, CLI verbs under the existing
cert tree, and the §7 fixture oracle as the acceptance tests. Start it
after PR #14475 (cert CLI overlap) settles. Live apply and live proof
remain undesigned on purpose, behind the #9155 production gate.
