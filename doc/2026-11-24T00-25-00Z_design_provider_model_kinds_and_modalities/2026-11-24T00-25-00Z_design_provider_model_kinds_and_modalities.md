# Design: provider/model purposes and input/output modality routing

**Date:** 2026-11-24 00:25 UTC
**Status:** Proposed design; no implementation or configuration change in this record.
**Tracking:** [code#10306](https://github.com/developing-today/code/issues/10306), under the design mill tracked by [code#9399](https://github.com/developing-today/code/issues/9399).
**Related design:** [`brief-capabilities-supplement.md`](../2026-11-13T09-26-00Z_design_pool_effort_system/brief-capabilities-supplement.md), especially §§0–3 and §8.

## 1. Problem and context

The router needs to distinguish _what operation a model can perform_ from
_what data it can receive or produce_. Without both distinctions, a route can
select a model that accepts the wrong request protocol or cannot consume the
request's attachments. A typed decision endpoint is not a chat endpoint; an
embedding model does not produce a chat completion; and an image-capable model
is not necessarily eligible for a text-only pool or vice versa.

The source issue records concrete failures and requirements:

- System One / Jev decision endpoints reject chat calls, while the historical
  `system1` route was only a name that resolved and did not itself supply a
  decision-call path.
- Voice, embedding, image generation, transcription, and reranking need
  distinct operation families. In particular, reranking is not embedding:
  it scores query/document pairs rather than producing reusable vectors.
- The capabilities supplement already specifies capability-set eligibility,
  per-capability failover, pool inheritance/wrappers, and provider translation.
  It also calls out the interaction with context-limited pools.
- The repository has since added `model_designations` in
  `.opencode/opencode.jsonc` (code#13675): model-level `purposes` and
  `capabilities`, source strings, and a static route-rules check over every
  model route. This is useful groundwork, not yet the complete request-time
  contract: declaration/checking alone does not prove that an incoming
  multimodal request is normalized, filtered at every fallback boundary, and
  delivered through a compatible provider API.

The immediate design goal is to complete and formalize that groundwork rather
than add an unrelated parallel metadata system. Existing configuration and the
supplement are the evidence base; this document does not claim that the
proposed runtime behavior exists today.

## 2. Goals and non-goals

### Goals

1. Make operation-family compatibility structural and checked before dispatch.
2. Represent input and output modalities independently, including combinations
   such as text+image input and text+audio output.
3. Filter ineligible models at every route, nested-pool, and class-fallback
   boundary; preserve per-capability failover rather than treating one
   modality's failure as another's.
4. Preserve context limits, provider-specific request translation, weights,
   ordering, quota/health gates, and existing route semantics for compatible
   requests.
5. Make metadata provenance and uncertainty auditable, using provider catalogs
   and models.dev without treating a catalog claim as live end-to-end proof.
6. Migrate existing seats incrementally, with decision-only routes as the first
   high-value kind-safety case.

### Non-goals

- Implementing new model endpoints, a general-purpose AI gateway, or all
  OpenAI/Claude/System One protocol families in this design-only change.
- Inferring capabilities by sending user content to candidate models.
- Treating `purpose` as a replacement for billing, quota, strength, context,
  transport, or harness-only feature metadata.
- Automatically enabling every modality a provider advertises, or changing
  route weights/order as part of metadata migration.
- Treating PDF as interchangeable with image: PDF/document parsing is a
  document-input feature and must not be inferred from image input alone.

## 3. Alternatives considered

| Option                                                 | Advantages                                                                                                                                                       | Costs / reason not chosen                                                                                                                                                                              |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Do nothing; rely on comments and provider errors       | No migration or router changes                                                                                                                                   | Keeps wrong-family routes expressible; errors occur after dispatch, may leak attachments to an incompatible provider, and cannot provide predictable fallback.                                         |
| One boolean `kind` per model                           | Simple validation and UI                                                                                                                                         | A single exclusive kind cannot express a model that supports both chat and decision, or providers exposing multiple API families. It also conflates operation with input/output data.                  |
| Capability flags only (for example `vision: true`)     | Small schema, familiar                                                                                                                                           | Does not distinguish chat from embedding/rerank/decision; booleans do not express required combinations, direction, document parsing, or pool policy.                                                  |
| Put all metadata on pools                              | Lets operators control routes locally                                                                                                                            | Duplicates model facts across pools, drifts as the same model is seated elsewhere, and cannot support catalog reconciliation. Keep pool declarations as _allow policy_, not the source of model facts. |
| Put all metadata on providers                          | Captures endpoint families                                                                                                                                       | A provider may serve models with different capabilities and model IDs may vary by endpoint/version. Provider endpoint support and model-specific capability claims must remain distinct.               |
| One shared model designation plus pool policy (chosen) | Extends existing `model_designations`; separates model facts, endpoint-family support, request requirements, and pool allow-lists; supports multi-purpose models | Needs a closed vocabulary, migration of unclassified routes, source reconciliation, and runtime enforcement across all route compositions.                                                             |

## 4. Chosen approach and contracts

### 4.1 Three independent dimensions

Retain `purposes` as a **set** on a model designation, not a singular `kind`:
`chat`, `decision`, `embed`, `rerank`, `recommend`, `transcribe`, `speak`,
`guard`, and future reviewed operation families. A model may list multiple
purposes only when there is evidence that each operation is served. A provider
declares which API families/transports it exposes; that declaration is not a
claim that every model under that provider supports each family.

Represent modalities directionally on each model: `input_modalities` and
`output_modalities`, drawn from a closed vocabulary (`text`, `image`, `video`,
`audio`, `document`). Keep protocol features such as real-time audio streaming
and file/path upload as separate transport/feature capabilities, not synonyms
for a modality. Model-specific operations and modalities are independent: for
example, an image input capability does not make a model an image generator.
Document/PDF support requires explicit document parsing evidence; it is not
derived from image support.

The existing `capabilities` field is a transition surface. During migration,
map its current values (`visual`, `pdf`, `audio-in`, `audio-bidir`, `tts`,
`stt`) through a single checked compatibility table into the directional model
and transport features. Do not keep two independently edited vocabularies.
Rename or replace that field only in an implementation slice after the
compatibility mapping and all consumers are tested.

### 4.2 Configuration ownership and precedence

- **Model designation:** one canonical record keyed by normalized catalog
  identity, with provider-specific IDs mapped to it. It contains purposes,
  directional modalities, optional context limits, evidence/source, checked
  time, and confidence/status. Keep an explicit override for provider-specific
  variants when the endpoint's actual behavior differs from the canonical
  family.
- **Provider endpoint family:** provider adapter declares the operation/API
  families it can translate and dispatch (chat/responses, embeddings,
  reranking, decision, speech, etc.), including required native knobs. One
  provider entry may expose multiple families; silently dropping a required
  knob is forbidden. Compatibility façade translation remains one layer, as
  required by the issue's Cohere example.
- **Pool policy:** a pool declares allowed purposes and an array of accepted
  _sets_ of input/output/transport requirements. Model facts flow upward when
  a pool has no policy. An explicit wrapper/pool policy narrows that branch;
  nested policies compose by intersection, never by widening a child. This
  preserves the supplement's `Luna text` wrapper example and prevents a
  text-only wrapper from inheriting image eligibility accidentally.
- **Request contract:** normalize the incoming API surface into one operation
  purpose plus required input modalities, requested output modalities, and
  transport features. A multi-part request requires every member of its
  capability set. Translation to the selected provider's native shape happens
  only after eligibility is established.

Precedence for conflicting evidence is: provider's current operation-specific
catalog/API evidence; models.dev record for the exact normalized model/version;
reviewed local override with source and timestamp. Provider evidence is
authoritative for available endpoint families; model-level capability claims
remain version-specific. A disagreement marks the record `needs_review` and
must not silently broaden routing eligibility. Ingestion is a candidate-data
process, not an automatic route-enablement mechanism.

### 4.3 Router and fallback enforcement

1. Normalize request requirements once at the routing boundary. Unknown
   operation is rejected before route expansion; an absent modality is not
   guessed from content bytes.
2. Before selection, intersect request requirements with the consuming pool's
   policy, each nested pool/wrapper policy, the model designation, and the
   provider adapter's supported operation family. Any mismatch makes that leaf
   ineligible (equivalent to walled for this request only).
3. Apply the same eligibility test when expanding class fallback routes and
   provider-route members. A fallback may not shed a required modality or
   change operation purpose. If no eligible leg remains, return a typed
   `no_eligible_route` error that identifies the missing requirement without
   echoing attachment contents.
4. Keep capability-local failures scoped to the affected operation/capability
   set; global failures such as quota/auth/route-construction failure retain
   their existing whole-leg semantics from the supplement.
5. Preserve per-target context caps such as `limited`: eligibility is checked
   before dispatch, then the selected target's effective context limit is
   applied. Context overflow is not repaired by selecting an ineligible
   modality fallback.
6. Record operation, normalized capability-set ID, eligibility disposition,
   and selected leaf in route-attempt telemetry. Do not record prompts,
   documents, image/audio bytes, or extracted content.

### 4.4 Security and capability analysis

Metadata is routing input, not a security boundary on its own. Only reviewed,
source-attributed claims may broaden routing; unknown or stale declarations
must not enable a non-text path. Attachments are passed only to a leaf proven
eligible for the exact requirement set and only through its adapter. PDF and
other document parsing is an explicit capability because parsers process
untrusted complex input. Telemetry uses opaque capability identifiers and
must never capture attachment payloads. Config validation rejects unknown
purpose/modality values and duplicate/conflicting normalized model identities.

## 5. Dependency-ordered build slices

Each slice is intended to fit one implementation lane/session. This design
does not authorize implementation; each slice should be tracked separately
after the design is accepted.

1. **Vocabulary and evidence inventory.** Define closed purpose, modality,
   transport-feature, and status enums; inventory current designations, model
   IDs, aliases, and route usages; publish the one-time legacy compatibility
   map. **Acceptance:** every currently designated entry maps without loss;
   duplicate/unknown values fail validation; inventory names all unclassified
   routed models and assigns an evidence source or an explicit unknown.
2. **Static config contract and diagnostics.** Validate model designation,
   provider-family, pool-policy, wrapper composition, and route/fallback
   compatibility. Keep runtime behavior unchanged in this slice.
   **Acceptance:** fixtures reject a decision-only model in chat, a rerank
   model in an embedding pool, a modality-incompatible fallback, unknown enum
   values, and widening nested wrappers; positive fixtures cover multi-purpose
   models and a narrowed text wrapper.
3. **Request normalization and eligibility engine.** Convert chat/messages,
   responses, embedding, rerank, decision, and supported media request shapes
   to the canonical contract; filter candidate leaves before dispatch at all
   composition boundaries. **Acceptance:** model-based tests prove that every
   returned leaf satisfies purpose, modality, transport, and context
   constraints; unsupported combinations return `no_eligible_route` without
   provider calls or payload-bearing logs.
4. **Provider adapters and façade families.** Add/extend family-specific
   adapter contracts and mechanical translation, preserving native-only knobs
   (including Cohere embed `input_type`); wire the minimum family needed for
   each admitted purpose. **Acceptance:** provider contract tests assert the
   exact translated request and prove unsupported knobs fail rather than
   disappear; chat and non-chat families share one provider identity without
   cross-family endpoint confusion.
5. **Decision-first migration and telemetry.** Migrate `system1`/decision
   designations, then embeddings/rerank and speech; finally classify visual,
   PDF, and remaining multimodal seats from evidence. Add capability-set
   telemetry and staged rollout gates. **Acceptance:** decision calls resolve
   only decision-capable adapters, chat cannot select decision-only leaves,
   every migrated route has a source, and sampled routed requests show the
   expected eligibility/selection without payload capture.

## 6. Rollout and rollback

Roll out in shadow/diagnostic mode first: compute eligibility alongside the
existing selector, record only requirement IDs and would-filter decisions, and
compare with route-attempt outcomes. Then enable hard filtering for one family
at a time, starting with decision-only seats because the existing failure is
structural and the model set is small. Next enable embeddings/rerank and
transcription/speech, followed by visual/document/video inputs. Keep the
existing route order, weights, quota gates, and `limited` context caps fixed
during each step so the effect is attributable.

For each stage, require static config validation, family contract tests,
representative end-to-end proof, and an observed no-payload telemetry review.
If the eligible set unexpectedly empties or a provider rejects a translated
request, disable the new family gate (or revert its designation/pool policy),
restore the previous route behavior for that family, and preserve the incident
evidence. Never roll back by routing a known incompatible attachment to a
text-only model. Existing route-attempt records and user data remain unchanged.

## 7. Verification plan

- **Schema/config:** closed-enum tests, source-required checks for non-chat
  designations, alias-collision tests, pool/wrapper composition tests, and
  `bin/check-opencode-route-rules.mjs` coverage for every pool and fallback.
- **Property/model tests:** for every selected leaf, assert requested purpose
  and each required input/output/transport feature are supported, pool policy
  is satisfied, nested restrictions never widen, and context limits hold.
  Generate combinations of request sets, model offers, and nested pool
  declarations; the oracle is the formal subset/intersection rule in §4.
- **Adapter contract tests:** assert native endpoint family, parameter
  preservation, media conversion, and errors for unsupported operations.
  Include a provider with multiple families and a model with both chat and
  decision purposes.
- **Failover tests:** capability-local failure advances only that request's
  eligible legs; quota/auth/route-construction failures retain existing
  global semantics; fallback expansion never drops requirements or repeats
  routes.
- **End-to-end live proof:** use non-sensitive synthetic text/image/audio and
  document fixtures against a designated test route, record exact provider,
  model, operation, and capability-set IDs, and confirm no content appears in
  telemetry. Live provider testing is separately authorized and is not part of
  the design task.
- **Rollout observation:** compare shadow decisions with actual outcomes and
  review empty-eligible-set rates, wrong-family rejections, and failover rates
  by operation before enabling each gate.

## 8. Open questions and defaults

1. **Vocabulary owner — Drew:** should the proposed `document` input be a
   modality or a separately typed content format (PDF, office documents)?
   **Default:** keep document as a distinct required feature, not implied by
   image; settle the final enum before slice 1 lands.
2. **Metadata owner — Drew:** should models.dev be the default source for
   model identity/capability data, or should provider live catalogs take
   precedence wherever they exist? **Default:** exact provider evidence wins
   for endpoint availability; models.dev is the normalized catalog baseline;
   conflicting records require review and do not broaden eligibility.
3. **Migration owner — Drew:** should an undesignated legacy model default to
   chat+text during migration, or become ineligible until reviewed?
   **Default:** static audit first; runtime migration must explicitly classify
   current routed models before fail-closed enforcement. After the migration
   gate, unknown models cannot serve non-text or non-chat requests.
4. **API contract owner — OpenCode maintainers:** which provider API families
   are required in the first adapter slice (Responses, native embedding,
   rerank, System One, speech)? **Default:** decision and embedding first,
   rerank next, then media families; no family is claimed complete without a
   contract test.
5. **Rollout owner — Drew:** which non-sensitive live test accounts/routes may
   be used for media end-to-end proof? **Default:** tests remain synthetic and
   opt-in; no customer/user attachment is used.

## 9. Record and tracking

This dated design is the design record for [code#10306](https://github.com/developing-today/code/issues/10306), which remains the tracking issue; its existing queue task is the delivery queue item for this design slice. No new implementation/config change is included here. The source issue should link this file (and the eventual merged revision) so subsequent slices can be issued against the settled contracts above.
