# Owner-inventory matcher (Layer 3)

`atlas_core::OwnerInventory` is a narrow component, not a second scanner
pipeline. The current Atlas search pipeline has no scan entry point for this
Layer 3 operation. A future internal-tier scanner should load the inventory
once at startup and call `scan_internal` only for content it has already
classified as internal tier. A load error is fatal for this detection layer;
callers must not report a clean Layer 3 scan when loading fails.

## Version 1 inventory

Store exactly one JSON document in Bitwarden Secrets Manager under a single
secret key. Configure only that key's name in
`ATLAS_OWNER_INVENTORY_SECRET_NAME`; `OwnerInventory::load_runtime` fetches its
value through Atlas's existing Bitwarden secret helper. Do not put inventory
values in environment variables, files, source control, fixtures based on real
people, logs, reports, or persistent scan artifacts.

```json
{
  "format": "atlas-owner-inventory/v1",
  "records": [{ "id": "opaque-record-01", "class": "address", "value": "synthetic example" }]
}
```

Each record has a unique opaque ASCII ID (`A-Z`, `a-z`, `0-9`, `_`, `-`), a
class (`address`, `phone`, `family_name`, or `account_id_fragment`), and a
non-empty literal value. Phone patterns compare digits only and require at
least seven digits; other classes use case-insensitive substring matching.
Findings contain only the class, opaque record ID, `high` severity, and
`quarantine_candidate` action. They never include a match or pattern value.
Malformed, absent, oversized, or unsupported inventory fails closed with a
value-free error.

## Single-source updates

Maintain and review the inventory only in the named secret-store value. An
authorized maintainer edits that one value in place, preserves the v1 schema,
uses stable opaque IDs, and validates the replacement using synthetic data
before publishing it. The repository contains no mirror or plaintext backup.
Operational real-data sweeps and counts-by-class reporting are separate
authorized work; neither is part of matcher implementation or its tests.
