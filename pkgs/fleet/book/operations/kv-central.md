# Running the `fleet kv` central plane

Hosts reach the shared `fleet kv` primary only through the hosted kv door
on `fleet-hosted`. They never connect to Turso directly. This page covers
wiring the door and the hosts, checking that it is ready, and backing up
and restoring a session's partition through it. It covers #13316,
acceptance 5 and the v1 operational integration. Design:
`doc/2026-11-23T12-45-00Z_design_fleet_kv` §4, §9, D011 and D014.

## Wire the door

| Setting                | Where                  | What it is                                                                                     |
| ---------------------- | ---------------------- | ---------------------------------------------------------------------------------------------- |
| `KV_LIBSQL_URL`        | `[vars]` of the Worker | The shared primary, `libsql://…` or `https://…`. When it is unset, the door answers 503.       |
| `KV_LIBSQL_AUTH_TOKEN` | Worker secret          | The primary's Turso token. Only the Worker holds it, and it is never given to hosts or agents. |

The door runs the schema migrations itself, from the catalogue, the first
time an isolate serves a request. A failed migration is retried on the next
request. Hosts never send DDL.

## Wire a host

```sh
export FLEET_KV_URL=https://<fleet-hosted>/v1/kv
export FLEET_KV_TOKEN_FILE=/run/fleet/kv-token   # mode 0600
```

The token file holds one of three credentials:

- **A person's RP token:** the caller reads and writes only as its own
  tenancy binding id. The session's launch identity `owner_user_id` must
  be that binding id.
- **An agent's RP token:** the same rule, and the token must name its lane
  and session.
- **A service token scoped to the `kv` door:** for a host daemon that
  sweeps for several users. It is trusted for any owner, but only inside
  its route scope.

A per-user Turso data-plane token is refused with
`data_plane_credential_refused`. Central access never uses one.

Namespace definitions and grants are admin statements. They need the
`kv-admin` role on an RP token, or a service token scoped to the `kv-admin`
door as well as `kv`.

## Define namespaces

A host can use a namespace only after it is defined on central. Run these
with an admin credential in `FLEET_KV_TOKEN_FILE`. They never open a local
store, so they work from a machine with no session.

```sh
fleet kv namespace define status --shared fleet-status
fleet kv namespace define notes --private --blob-threshold 65536
fleet kv namespace list
```

- `--shared SCOPE` puts every agent in one shared scope. `--private` gives
  each owner and context its own scope.
- Defining a namespace again keeps the settings you leave out and raises
  its policy epoch.
- Changing a namespace between shared and private is refused with
  `namespace_scope_change`, because its existing keys would be stranded.
- Hosts pick up a change on their next `fleet kv sync`.

## Collect unreferenced blobs

A blob that no key references any more is collected in two steps. Run one
pass per partition, with the same admin credential:

```sh
fleet kv blob gc eu/standard
```

- The pass marks every published blob in the partition that nothing
  references, and clears a mark once a reference lands or the object is gone.
- It fences the delete of each blob that stayed unreferenced for the grace,
  24 hours by default (`--grace-ms` changes it).
- It prints `partition=… marked=N cleared=N deleted=N`, then one
  `deleted content=DIGEST epoch=N` line per fenced delete. The object bytes
  stay in the store; this pass only retires the central record.

## Check readiness

- `GET /health` reports `kv.primary_configured`. It is a boolean and
  never the URL.
- `fleet kv sync` from a wired host pulls the namespace definitions
  through the door. It exits 0 when the door, the credential and the
  primary all work.

## What the door refuses

The door forwards only the statements in
`pkgs/fleet/hosted-worker/src/kv-catalog.json`, matched by exact text.
That file is generated from `pkgs/fleet/crates/fleet-kv/src/catalog.rs`,
and a test keeps it equal. After a client statement changes, regenerate
the file with
`FLEET_KV_WRITE_CATALOG=1 cargo test -p fleet-kv central_catalog`.

| Code                        | Status | Meaning                                                                                                                |
| --------------------------- | ------ | ---------------------------------------------------------------------------------------------------------------------- |
| `kv_primary_not_configured` | 503    | No `KV_LIBSQL_URL`.                                                                                                    |
| `kv_pipeline_malformed`     | 400    | The body is not the client's one-transaction batch, or an argument is malformed.                                       |
| `kv_statement_not_allowed`  | 403    | A statement is not in the catalogue.                                                                                   |
| `kv_admin_required`         | 403    | A namespace or grant change without `kv-admin`.                                                                        |
| `kv_identity_refused`       | 403    | An owner or writer other than the caller.                                                                              |
| `kv_scope_refused`          | 403    | The scope is neither the namespace's shared scope nor the caller's own private scope, or the namespace is not defined. |
| `kv_primary_failed`         | 502    | The primary did not answer.                                                                                            |
| `service_token_route_scope` | 403    | The service token is not scoped to `kv`.                                                                               |

## Back up and restore a partition

Each session's partition is archived to the primary through the door. The
local host snapshots in [Backing up `fleet kv`](kv-backup.md) are a
separate mechanism.

- `fleet kv sync` sweeps this session's intents. It also writes a
  checkpoint when the last one is 15 minutes old or there is none.
- `fleet kv backup` syncs, then writes a checkpoint now.
- `fleet kv restore --archive` on a fresh host adopts the newest
  checkpoint. It never overwrites a key the host already has history for,
  and it then sweeps.

A checkpoint is a 15-minute online target, not a recovery point. Two kinds
of write are lost if the disk is lost:

- writes made after the checkpoint;
- writes made while the primary was unreachable.

Values over the inline threshold and working files are listed as
`unprotected … protection_unavailable_needs_phase3`, because Phase 3
carries blobs.

## Not covered yet

- Group databases from `kv_grants` are not reachable through the door. A
  group write placed by a grant needs a provisioned database, and none is
  wired to `fleet-hosted`.
- Blobs (Phase 3, #13318).
