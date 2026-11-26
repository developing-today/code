# `command` family

Contract owner: DST-01 / CLI-01 (#9451), `fleet-queue/src/work/contracts.rs`.

`cases.json` holds one `fleet.command/v1` envelope, its JCS intent
DTO and request digest, two digest invariants (a retry with a new
operation id, delivery and dry-run keeps the digest; a changed
precondition changes it), one `fleet.result/v1` envelope, and
negative patches with the typed error each must produce. A patch is
merged into the golden envelope at the top level; `remove` drops a
field.
