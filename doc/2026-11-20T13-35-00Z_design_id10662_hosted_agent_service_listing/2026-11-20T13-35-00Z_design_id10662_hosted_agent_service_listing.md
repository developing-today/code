# Hosted agent service listings: design record for v4 step 1

- **Issue:** [#10662](https://github.com/developing-today/code/issues/10662), under [#6303](https://github.com/developing-today/code/issues/6303).
- **Status:** design proposal. The inherited requirements below come from the [v4 outline](../2026-11-16_design_id6303_hosted_agent_services_v4_outline.md#21-the-registry-agent-programs-as-listable-services); the named fields, ordering and lifecycle mechanics are recommendations for review, not implemented behavior.
- **Scope:** signed listing, versioning, suspension and delisting evidence, candidate discovery. No change to hiring, authorization, world policy, hosting, runtime activation, cryptographic implementation or deployment. No board example depends on #10049.

## Boundary: a service listing is not a bundle listing

The [existing network program registry](../../pkgs/id/crates/id-worlds/src/program_registry_net.rs) serves author/name-filtered lists of [`BundleRecord` values](../../pkgs/id/crates/id-worlds/src/program_bundle.rs), fetches by content hash and accepts author-signed bundle withdrawal. A bundle record authenticates the author's exact manifest statement. The manifest binds author, name, version and payload hash; a fetched bundle is verified on the existing bundle path. [Gallery admission](../../pkgs/id/crates/id-worlds/src/program_gallery_admission.rs) is endpoint-local: its public transparency list records removal, refusal or reinstatement, not a revocation of the bundle's signature or its copies elsewhere. Existing artifact behavior and [#11727](https://github.com/developing-today/code/pull/11727) stay in that layer.

A hosted listing answers a different question: **which agent, under which owner and operator, advertises a service using a particular immutable program release?** Reference that release; do not re-publish its bytes or copy its signed manifest into a second artifact registry. One bundle may back multiple independently operated listings. Removing one listing must not mutate, delete or withdraw the bundle, or change another listing.

From v4 §2.1 and decision 1: listings are signed and versioned, bind agent identity, program manifest, willing roles, operator and availability, and are discoverable but not endorsed. The owner or operator may suspend to stop new hires and begin the prescribed drain; an individual registry operator may publicly delist for abuse with an evidence trail. World owners still decide whom to hire through their existing world mechanisms. Infrastructure obtains no world role or delegated authority from a listing or from indexing it.

## Proposed record and identity

The table names logical fields, not a serialization or a finalized schema.

| Field | Meaning |
| --- | --- |
| `schema_version` | How to interpret and verify this record class. An unknown schema is not an eligible candidate; it is not coerced into a known version. |
| `listing_id` | Stable opaque identifier scoped by its originating owner identity; registry endpoint and bundle hash are not the service identity. |
| `revision`, `previous_revision_ref` | Monotonic revision and a reference binding the exact previous signed record; absent predecessor only at creation. This is distinct from the artifact version. |
| `agent_key`, `owner_ref`, `operator_ref` | Agent public identity and accountable principals. Artifact author, agent owner, service operator, registry operator and hiring world owner are not presumed interchangeable. The record does not confer control of any key. |
| `bundle_ref` | Immutable author/name/version/content-hash tuple; candidate validation must match all four to a verified artifact manifest. Artifact lookup location is a hint, not provenance. |
| `service_manifest` | Bounded purpose, advertised world surfaces and resource profile. Distinct from the author's artifact manifest; a service claim is not permission or proof of measured resources. |
| `willing_roles` | Bounded list of world role selectors understood in context. A match is only a willingness claim, never a role grant or authorization to hire. |
| `availability` | Advertised `always_on`, `scheduled` or `on_demand` terms, with schedule reference/time basis where relevant. Not a claim of current health or presence. |
| `issued_at`, validity | Attributable issuance time and bounded validity interval or freshness boundary. Time metadata does not substitute for revision ordering. Policy limits and time format remain open. |
| Provenance proof | Verifiable authorship and owner/operator attribution binding the exact record under a distinct listing context. Verification uses the project identity model; no new trust anchor is granted by indexing. |

**Signing recommendation.** Bind the whole record including version, predecessor, principals, release link and terms. Require evidence that the listing is authorized by its agent owner and attributable to its advertised operator; a valid artifact signature verifies the artifact author but cannot satisfy listing authorization. A listing proof must not be accepted as an artifact, lifecycle event, hire or world-action proof. Ambiguous or missing identity binding is not eligible. Which existing authority proof, signers, encoding and algorithms are appropriate is an owner/schema decision for a later implementation record; this design neither issues nor changes credentials or delegation.

**Versioning recommendation.** Verify each revision and preserve older versions for audit. Accept an identical replay idempotently; reject an older revision as a replacement for a known head. Treat two different records claiming the same identity and revision, or a head without its required predecessor evidence, as unresolved, not “pick by timestamp.” A different artifact release or change in advertised terms uses a new listing revision, never an in-place rewrite. Do not implicitly reactivate a suspended or delisted entry by publishing a revision. Proposed identity rule: an agent identity or owner boundary change creates a different listing identity; operator succession and proof/rotation mechanics remain open. No cross-registry consensus or global completeness is assumed.

## Lifecycle and audit trail

| Fact | Authority inherited from v4 | Proposed record and effect |
| --- | --- | --- |
| Publish / revise | Listing provenance identifies who listed it and under which operator. | Verified signed revision as above; no hire, access or admission is implied. |
| Suspend | Listing owner or its operator. | Append an authenticated event linked to the listing and observed revision. Treat suspension as applying to the listing identity so a newer revision cannot bypass it; exclude from new candidates and record a drain signal for the separately specified hire lifecycle. |
| Delist for abuse | Registry operator for that registry. | Append an authenticated, registry-scoped event linked to the listing and observed revision; publish a reason class and safe evidence reference. Exclude there; do not imply a network-wide revocation, silently remove history or delete the artifact. No runtime action is specified here. |
| Restore eligibility | Not settled by the outline. | If supported, use a distinct attributable event referencing the exact prior event; recommence only after all applicable blocks are resolved. Neither a revision nor an event from another registry overrides a block. Actor authorization, review and restoration policy are deferred. |

Proposed event fields: record class/version, event identity, listing identity, observed revision reference, actor identity and authority scope, recorded time, disposition and safe reason/evidence references. Order lifecycle events within the appropriate authority scope using verified history; do not compare unrelated registry clocks to override a suspension. If required listing or lifecycle proof is missing or contradictory, do not present the listing as currently eligible. Restricted underlying audit content can remain unavailable without, by itself, making a verified lifecycle receipt unresolved. Retain prior events so reviewers can distinguish reversal from erasure.

The outline names worlds served, completed and expired claims, and review outcomes as **audit references**, not as public claims of quality. A reference identifies an attributable record; it neither grants access nor establishes the registry's endorsement or the truth of an unverified claim. A public delisting receipt can include a non-sensitive reason class and opaque evidence reference. Private world identities, participant information, private review notes, credentials and underlying evidence are not copied into the public record. Evidence content stays governed by its existing access rules; inaccessible evidence is labeled unavailable, not silently treated as verified or false. Use coarse reason/class metadata or omit an otherwise revealing reference where policy requires.

## Candidate query and result

A world queries a chosen registry for candidates; other registries remain independent. The proposed logical request includes optional role selector, required advertised surfaces, availability mode and an artifact or agent selector, plus a bounded page size and an optional opaque continuation cursor. Omit private world context if not needed. A match covers all requested surfaces and the requested role/mode; it does not imply current availability, compatibility with local trust policy or an invitation to act.

For each item return the immutable listing/revision reference, signed listing provenance, verified artifact identity and applicable lifecycle/policy provenance or status. Do not return bundles or underlying audit content in this query. Candidate projection includes only verified current listings matching filters with no applicable suspension/delisting/admission block in the view consulted. If proof, history, freshness or session view is unresolved, exclude it from candidates and distinguish an incomplete/unavailable result from a verified empty page. Recheck at any separately authorized later decision; this read is not a reservation, hire or permission to take action.

Bound list traversal with finite request dimensions, page size, response size and work limits; apply the filters and budgets before materializing results. Recommend stable order by scoped listing identity within a snapshot, cursor tied to the endpoint/query/snapshot, and explicit invalid/expired cursor outcomes rather than silently restarting. Return the view scope and observation/freshness information, a next cursor if available and an explicit coverage indicator if incomplete. No fixed numeric caps, wire names, transport or federation policy are committed here. Client verification must not take a registry assertion or mere absence of evidence as proof of global state.

## Offline design example

Synthetic values below are symbolic identifiers and outcomes, not keys, proofs, endpoints, real people, wire schema or executable fixtures. The listings share an artifact to demonstrate independent service disposition. Time and proof are abstracted as a verified synthetic view.

```json
{
  "bundle_refs": ["bundle-illustrative-r7"],
  "query": {
    "registry": "registry-example-a",
    "role": "reviewer",
    "surfaces": ["documents"],
    "availability": "scheduled",
    "limit": 2
  },
  "listings": [
    {"id": "owner-a/listing-a", "revision": 1, "bundle": "bundle-illustrative-r7", "roles": ["reviewer"], "surfaces": ["documents"], "availability": "scheduled"},
    {"id": "owner-b/listing-b", "revision": 2, "bundle": "bundle-illustrative-r7", "roles": ["reviewer"], "surfaces": ["documents"], "availability": "scheduled"},
    {"id": "owner-c/listing-c", "revision": 1, "bundle": "bundle-illustrative-r7", "roles": ["reviewer"], "surfaces": ["documents"], "availability": "scheduled"},
    {"id": "owner-d/listing-d", "revision": 1, "bundle": "bundle-illustrative-r7", "roles": ["reviewer"], "surfaces": ["documents"], "availability": "on_demand"}
  ],
  "events": [
    {"id": "event-s", "action": "suspend", "listing": "owner-b/listing-b", "observed_revision": 1, "scope": "listing", "audit_ref": "receipt-s"},
    {"id": "event-d", "action": "delist", "listing": "owner-c/listing-c", "observed_revision": 1, "scope": "registry-example-a", "audit_ref": "receipt-d"}
  ],
  "expected": {
    "candidates": ["owner-a/listing-a"],
    "excluded": {
      "owner-b/listing-b": "suspended",
      "owner-c/listing-c": "registry-delisted",
      "owner-d/listing-d": "availability-mismatch"
    },
    "bundle_unchanged": true
  }
}
```

The first listing matches. The second remains suspended even though its current revision has advanced. The third is excluded by this registry's decision, not declared unavailable anywhere else. The fourth does not meet the advertised schedule filter. All keep the same artifact reference; the receipts do not disclose any underlying audit content.

## Decisions deferred

- [V4 §5 Q3](../2026-11-16_design_id6303_hosted_agent_services_v4_outline.md#5-open-questions-needing-drews-ruling): public backstop third-party listing/hire posture remains Drew's ruling; it gates the later public-backstop step, not this design. Do not import the artifact gallery's default admission posture.
- Existing identity proof composition, canonical record/hash/signature encoding, field representation and bounds, authority verification and recovery/rotation details, lifecycle restoration and propagation/freshness behavior need separate owner and implementation review.
- No decision here on hire, attach, delegation, presence, drain mechanics, quota accounting, host behavior, or world review and acceptance authority. No #10049-dependent board examples.

## References

- [V4 discovery scope, decision 1 and next-step boundaries](../2026-11-16_design_id6303_hosted_agent_services_v4_outline.md#6-next-step-breakdown-filed-as-native-sub-issues-of-6303)
- [Program bundle verification and record shape](../../pkgs/id/crates/id-worlds/src/program_bundle.rs)
- [Artifact registry operations and bounds](../../pkgs/id/crates/id-worlds/src/program_registry_net.rs)
- [Gallery admission and endpoint-local transparency](../../pkgs/id/crates/id-worlds/src/program_gallery_admission.rs)
