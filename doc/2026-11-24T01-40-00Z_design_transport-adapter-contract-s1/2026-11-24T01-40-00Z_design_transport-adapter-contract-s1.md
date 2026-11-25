# Transport adapter contract — S1 declaration and registry

**Status:** implementation record for #13929 S1; based on the head design's §4.
**Scope:** normalize the contract into a machine-readable schema and initial registry. This record preserves the contract as adopted for the first four existing instances; it does not claim that their behavior has been changed or live-conformance-tested.

## Contract

Each adapter has one versioned declaration with the same fields, regardless of protocol:

- **Identity and shape:** `name`, integer `version`, and `shape` (`transport` or `proxy`). A transport carries stack messages end-to-end; a proxy terminates at an external system and must declare credential custody, translation boundary, platform trust, and its authority ceiling.
- **Addressing:** carrier-native address and the explicit mapping to fleet-CA / ID identity. Carrier metadata is not an identity proof.
- **Independent levels:** `levels.send` and `levels.receive` each use `none`, `read`, `bidirectional`, `write-capable`, or `sync-equivalent`. These are per-direction capability ceilings, not a claim that both directions use one connection. `write-capable` is reserved for state-changing actions and therefore requires an explicit send gate.
- **Delivery:** guarantee (`at-most-once`, `at-least-once`, `best-effort`, or `not-applicable`), ordering, causal-metadata preservation, latency, size limit, and offline behavior. The declaration records limits; it does not make a carrier more reliable.
- **Identity, authorization, and metering:** identity binding (`cryptographic`, `mapped`, `none`), send/receive capability gates and revocation path, plus counted units and counter location.
- **Evidence:** each registry entry cites the issue/design source for the recorded facts. Unknown or provider-specific limits are stated as such instead of inventing a universal guarantee.

Routes remain separate from adapter declarations: a route composes one send adapter, one receive adapter, and an address mapping. Envelopes carry an explicit reply route; a failed send does not imply a failed receive. This slice establishes the declaration contract only, not routing-core behavior.

## Initial registry

`config/transport-adapters/registry.json` applies the exact schema to the bridge, mcpx door, email bootstrap, and sneakernet bundle. No per-adapter schema exceptions are used. The existing evidence describes configuration/capability boundaries, not new runtime integrations. In particular, `best-effort` email and `sync-equivalent` sneakernet are declared with their store-and-forward, revocation, and replay limitations visible.

The JSON Schema is Draft 2020-12. CI validates the complete registry on pull requests touching the schema/registry and on pushes to `main`. The validator is deliberately external to the application runtime; this slice does not introduce a transport library dependency.

## Security and compatibility

Declarations are descriptive and do not grant authority. Consumers must enforce their declared levels and gates. Proxy custody fields are mandatory only for `shape: proxy`; transport declarations may not claim them. Identity claims must be authenticated independently of carrier address. Revocation for offline paths is checked at receipt/import time and may be delayed until the path reconnects. The registry is additive and does not change any bridge, mcpx, email, or bundle behavior.

## Deferred to later slices

S2 owns route composition, reply-route envelopes, direction-specific level enforcement, and downgrade tests. S3–S8 own behavior-preserving formalization and live conformance proofs for the existing and new adapters. Open questions Q1–Q5 remain with the deciders named in #13929 §9; this schema does not settle them.

## Verification

The CI oracle is the JSON Schema 2020-12 validator checking `registry.json` against `schema.json`; the expected count is four adapter entries, with zero schema errors. The workflow runs this same command on PRs and `main`:

```sh
nix shell nixpkgs#check-jsonschema -c check-jsonschema --schemafile config/transport-adapters/schema.json config/transport-adapters/registry.json
```

<!-- pm-metadata:start -->
```json
{
  "pm_meta": 1,
  "provenance": {
    "queue_id": 21620,
    "lane": "oc-code-05",
    "worktree": "oc-code-05-13929-20261124T013820Z-4afdc35e",
    "created_at": "2026-11-24T01:40:00Z"
  }
}
```
<!-- pm-metadata:end -->
