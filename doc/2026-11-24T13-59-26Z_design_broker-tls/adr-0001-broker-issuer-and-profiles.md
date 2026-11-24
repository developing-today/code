# ADR-0001 — Broker TLS: issuer set and leaf profiles

- **Status:** proposed with the #13685 design record (2026-11-24); takes effect
  when the design is accepted. Design only — this ADR authorises no issuance.
- **Scope:** developing-today/code#13685; program #9155; custody #9158;
  publication #9164; companion hierarchy #13658.
- **Companion record:** `2026-11-24T13-59-26Z_design_broker-tls.md` (§4 carries
  the application of this decision; §5 the migration it feeds).

## Context

The fleet brokers (secrets broker, queue broker, laptop-agent) need TLS
identities from the program CA. Three candidate issuers exist in the estate,
and the `fleet-cert` profile registry needs broker-specific leaf profiles if
broker leaves are not to be indistinguishable from generic service leaves:

1. The fleet CA's **Base issuer set** — the program's internal-infrastructure
   issuer set, with independent A/B per-algorithm partitions and direct issuer
   cross-signatures (#9164).
2. The fleet CA's **Internal Web issuer set** — profiled for internal web
   services, with the Web leaf kit (90-day leaves carrying web SANs).
3. The **IdP CA** of #13658 — a live classical (P-384 root → issuing
   intermediate) hierarchy built for the OIDC/SAML identity provider, whose
   relying parties are today's SAML/OIDC stacks, kept by Drew's 2026-11-23
   ruling and later backed into the full certificate design via its
   adopt/cross-sign path.

## Decision

1. **Broker leaves issue from the fleet CA's Base issuer set.** Both A/B
   partitions are eligible issuers for broker leaves; brokers verify chains
   into both partitions. Normal issuance is from the active partition;
   failover is a designed behaviour of the migration (design record §8).
2. **Two new leaf profiles** join the `fleet-cert` registry, derived from the
   existing `server` / `client` profiles (EC P-256; `serverAuth` /
   `clientAuth`; digitalSignature):
   - `broker-server` — for a broker's TCP server leg.
   - `broker-client` — for a fleet component dialling a broker.
     Profile deltas: **90-day default validity** (spec §5.4 leaf default;
     automated renewal is part of the profile contract), **mandatory SAN set**
     (the component's internal DNS name(s) + its tailnet IP as an IP SAN; no
     wildcards, no public names), and **CRL-only revocation checking** (a CDP
     is embedded only where #9164's real-signed-CRL rule is satisfied; no OCSP
     URL until a functional Base-set responder exists).
3. **Private keys are generated on the component's host** and never move;
   issuance is CSR-based against an explicitly passed signer source, per
   `fleet-cert`'s existing rail and #9163/#9158 custody.

## Alternatives considered

- **Internal Web issuer set.** Rejected: its leaf profile is web-service
  shaped (web SANs, public-name adjacency in the reference kit). Brokers are
  not web services; issuing broker leaves from it would blur the set's
  relying-party story and inherit a profile that must then be excepted in
  every broker rule.
- **IdP CA (#13658).** Rejected: its relying parties are SAML/OIDC stacks and
  its keys exist to sign identity assertions and IdP metadata, under that
  story's own authority and gates. Making the fleet control plane a relying
  party of the IdP hierarchy would couple broker availability to a hierarchy
  whose convergence into the program design is still ahead of it, and would
  spend a signing hierarchy on a purpose it was not cut for. Drew's ruling
  keeps the IdP CA growing toward the full design; broker TLS joins the full
  design directly instead.
- **Self-signed per-broker certificates with pinning.** Rejected: it is the
  side channel #13685 explicitly forbids for trust distribution, it has no
  revocation or rollover story, and it would strand the brokers off the
  program CA permanently while looking finished.
- **Public ACME issuance.** Rejected: wrong trust domain. Broker names are
  internal, their endpoints are tailnet-addressed, and public CT exposure of
  internal control-plane names is a cost with no relying party to justify it.
- **Adopt SPIFFE/SPIRE identities now.** Deferred, not rejected: spec §27.3
  items 1 and 5 keep workload-identity frameworks as a studied track. The
  Base-set leaf identities chosen here (named component, internal DNS +
  tailnet IP SANs) map onto SPIFFE IDs mechanically if that track lands.

## Consequences

- `fleet-cert` gains two profiles and a broker SAN policy (slice BT-1); no
  other crate changes follow from the issuer choice itself.
- Broker trust bundles are the program's classical compatibility bundles
  (§3.5) until ML-DSA TLS verification exists end to end; the cross-signed
  issuer graph is the mechanism that admits an ML-DSA broker partition later
  without new roots (design record §4 records the re-check as a BT-1
  acceptance item).
- The IdP CA's backing-in path (#13658) is unaffected: no new relying party,
  no new cross-sign requirement is created by this decision.
- If a future record moves broker issuance to a dedicated issuer set (the
  spec's signing-graph modes leave room for later sets), this ADR is
  superseded by that record; leaves issued under it remain valid to their
  natural 90-day expiry, so the cost of changing this decision is bounded
  by one leaf lifetime.
