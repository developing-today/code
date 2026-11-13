# Catalog and variant component

## Contract and decisions
Use canonical immutable IDs independently of mutable names. Parent is current display placement; family IDs are persistent lineage, not ACL authority. A split creates only new group IDs, leaves existing items/comments/history intact, keeps empty groups for explicit archival, and supports a nested destination parent. Existing names require explicit allowDuplicateName for newly created groups. Graph mutations stage a clone, validate ancestry and allocations, and commit only once. Dedicated server operations call variantPlan with server IDs; generic snapshot writes must call assertVariantMetadataUnchanged. Before any read, the authorization service projects permitted objects before resolving family associations; hidden/archived are UI states, not access controls.

Catalog imports are preview-first, no network. Canonical URL plus explicit SKU permits exact-match dedupe; title matches are suggestions only. Amazon region and ASIN remain part of identity. Explicit variant SKU differences never collapse. Unrecognized links retain meaningful query parameters; only known tracking parameters are stripped. Pasted source provenance is marked unverified and date-stamped, with original pasted input retained only in the user's reviewed evidence flow, not secretly transmitted. No prices, ratings, compatibility or product attributes are inferred from title alone. Invalid links and credential-looking content are rejected/warned. New item UUID allocation is atomic.

## Integration
- Add public/catalog.js before app.js in build and HTML.
- Quick paste: catalogParsePaste(text) => {unique,duplicates,possible,warnings}. Present preview and explicit import confirmation. catalogImportPlan(state, selectedRows, uid) integrates selections inside normal versioned apply.
- variantsDialog retains previous API. Optional server endpoint should accept ids/options/version, call worker/variant-plan.mjs variantPlan(state,ids,options,serverUUID), then store under normal optimistic transaction.
- worker/variant-graph.mjs validateVariantGraph(items) rejects invalid parent references/cycles/duplicate IDs; assertVariantMetadataUnchanged(before,after) protects generic snapshots from forged family fields. Legacy missing family metadata remains valid. Dedicated operation may change association metadata after permission checks.
- Pure server/client planner parity is enforced by tests/catalog.mjs.

## Requirement coverage and limits
BA-014 intake: helpers complete; UI integration required. BA-019/021/023/037/041/042: stable nested variant planning, empty-group preservation, name exception and split associations implemented. BA-026 typed metadata normalization supports string/JSON/pairs/dictionaries but arbitrary nested values are retained, not flattened destructively. BA-046 international text and control stripping for importer; not a global sanitation policy. BA-001 original cart access remains unavailable; this is not automatic Amazon cart recovery. BA-002–013 research/medical compatibility remains separate. BA-028 per-membership ordering and BA-032 text-range annotations remain outside this component. Full-server authorization and history transaction are integration responsibilities. Unknown parent legacy data needs an explicit repair preview, never silent dropping.

## Tests
node tests/variants.mjs; node tests/catalog.mjs. Cases: split/regroup IDs, retained lineage, descendant cycles, mixed ancestor selection, allocation failure atomicity, duplicate-name confirmation, nested destination, international paste, credentials and URL rejection, exact canonical URL/variant dedupe, title-only ambiguity, malformed graph and generic association forgery.
