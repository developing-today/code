# Sync, replication and FileOps

## Discovery: gossip, mDNS, DHT, bootstrap

A node finds peers four ways, and each way has a literal switch on `id serve`.

Gossip is the default wide-area mechanism. Peers announce on a gossip topic and hear announcements from others. The topic fallback constant is `id-peer-discovery-v1` and the topic-secret fallback constant is `id-public-discovery-v1`. The operative defaults normally come from the embedded `defaults.json` (shipped content: mode `append`, topic `id-peer-discovery-v1`, topic secret `id-public-discovery-v1`, plus optional bootstrap and web binding entries), and `--no-default-topic` makes serve fall back to the hardcoded constants. `--topic` and `--topic-secret` override. `--no-gossip` turns gossip discovery and announce off; `--gossip` forces it on, and the two flags conflict. Disagreement CLI-6: the `--topic` help text presents the constant as "the default"; the constant is the fallback, and `defaults.json` is the operative source on a stock serve.

mDNS is local discovery, on by default. `--no-mdns` disables it on both `serve` and the remote file commands.

The DHT is mainline DHT address lookup and publishing. It is off by default. `id serve --dht` enables it, and the lookup code lives in `id-sync` behind the `dht` cargo feature; the flag exists in the parser regardless of the feature. Disagreement DES-6: the older documents named three different discovery mechanisms (ARCHITECTURE.md said mDNS plus Iroh's DHT; the 2026-03-22 peer-discovery record says iroh-gossip plus a distributed topic tracker; the README said only local discovery). The shipped flags above are the code's answer: gossip, mDNS and opt-in DHT coexist. The README and `ARCHITECTURE.md` now state that same answer, so DES-6 is reconciled in those documents; the 2026-03-22 record remains as history.

Bootstrap nodes seed the first contacts: `--bootstrap <IDS>` takes comma-separated node IDs and is repeatable, and `--no-default-bootstrap` ignores bootstrap IDs from `defaults.json`. Relays carry traffic when direct connections fail; `--no-relay` on serve and on the remote commands means direct connections only.

`id peers` reads the result. Its default mode is RPC-then-gossip against a running serve; `--gossip` restricts it to gossip and `--rpc` to RPC. Defaults: `--depth` is `1`, `--max-peers` is `1000`, `--timeout` is `30` seconds.

## The meta protocol and log sync

Commands reach a store through the meta protocol. The ALPN is `/iroh-meta/3` (`META_ALPN = b"/iroh-meta/3"` in `id-sync`). The wire half — `MetaRequest` and `MetaResponse` — lives in `id-sync::protocol`. The facade-facing surface (`MetaProtocol`, `MetaClient`, `FindMatch`, `TaggedMatch`, `MatchKind`) lives in `id-cli`. `MetaClient` chooses between a running serve (detected by the `.iroh-serve.lock` file in the data directory) and the in-process local node, which is why the same `id get` command works with or without a serve running.

Event replication runs on log sync, in `id-sync::log_sync`: p2panda log-sync sessions with a range-sync bridge, exposed as `SyncProtocol`. Ingest of a replicated event ends in one of five outcomes, the same five the hosted bridge records: accepted, duplicate, fork, waiting or refused. Writer sets gate ingest: an event from an author outside the object's writer set is not accepted into that object's projection (see [Identity, the directory and groups](identity-directory.md)).

Friend traffic runs on envelopes. The envelope ALPN is `/id-envelope/1`. The outbox is an append-only journal (`envelope_outbox::Outbox` in `id-sync`), replayed on open, so an undelivered envelope survives a restart. Signed artifacts travel the same layer: four kinds, `friend_remove`, `home_declared`, `document` and `tag`, each a signed body with author, sequence, previous hash, kind and body. Signed announcements (`announcement::{Announcement, SignedAnnouncement}`) carry peer announcements with signature verification.

Federation is a module family inside `id-sync`: `federation` with record join, record store and exchange networking, plus slices covering key lifecycle and revocation, replay guard, signature suites, signer binding, rejection events, conflict reports, sync scope, anti-entropy, peer priority, fanout budget, query grants and privacy, signed receipts, request identity, result dedup, loop inform, routing state, topology probe, takeover response, peer classes and the ingest gate. The federated query fan-out and peer trust class designs that extend this family are design records, carried in [Design: sync, replication and FileOps](../design/sync-fileops.md). One design/code distinction matters here: the design records describe a single `/id-sync/1` handshake for sync traffic. The shipped ALPNs in the code surveys are `/iroh-meta/3` for the meta protocol, `/id-envelope/1` for envelopes and `/id-program-registry/1` for the program registry. The `/id-sync/1` split is design material, not a shipped wire name.

## FileOps: one implementation, three rules that decide outcomes

FileOps (`crates/id-sync/src/fileops.rs`) is the single implementation of put, rename, copy, delete and transfer for the CLI, the REPL, the meta protocol and the web layer. A file is a name bound to an immutable blob hash, plus tag-store metadata (see [The file core: content addressing, names and tags](file-core.md)). Three rules decide the outcomes users see:

Rule 1 — archives belong to their owner, and only the owner takes them away. A rename or replace archives the previous binding as `{name}.archive.{unix-ts}` (collision-bumped, claimed under an in-process lock). The archive records `archive.owner`; the live name records `archive.rename` or `archive.replace`. There is no per-archive delete. Deleting the owner deletes its archives with it.

Rule 2 — delete is a hard delete of the name, and blob bytes outlive it. `FileOps::delete` removes the name, its metadata and its owned archives in one operation. The blob bytes stay in the store until garbage collection, because another name or a hash-only holder can still reference the same hash. The web layer's delete is a different operation built on top: a soft delete with a matching restore (`POST /api/delete`, `POST /api/restore`, `POST /api/hard-delete` for the permanent form).

Rule 3 — transfer moves bytes server to server, never through the client. `id send <SOURCE_NODE> <NAME> --to <DEST_NODE>` instructs the source server to push the file to the destination with the source server's own identity; no bytes pass through the client process. The web form is `POST /api/files/send` (#5346), and the library form is `id_sync::transfer` (server-to-server file transfer, #5345), re-exported at `id::transfer`. `--as <NAME>` names the file at the destination, and `--replace` archives the old destination file first, under Rule 1.

A note on numbering: the three rules above are stated in the order this chapter presents the FileOps survey material (archive ownership, delete semantics, transfer path). The surveys attest each rule's substance — archive ownership and the absence of per-archive delete, hard delete with blob survival, and the server-to-server push — in web-config survey §5.4 and CLI survey §1.4. A numbered Rule 1–3 list inside `fileops.rs` itself is **unverified** against the survey sources; the laptop checkout was unreachable during drafting, so the in-source numbering could not be checked.

Copy and rename complete the operation set. Both refresh the identity metadata keys `name`, `file` and `path` on the destination. Rename archives the source binding under Rule 1 when the destination already holds a file; the exact archive-on-rename trigger is recorded in the web-config survey as the rename handler's optional archive of the original or replaced file (`POST /api/rename`).

## Replica and mirror

Two commands keep a second copy of a world without hosting it.

`id world mirror <NODE> --capability <C>` runs a local replica endpoint that accepts sync connections. It is not a world host (the F13.8 boundary in [Worlds and the Roc guest runtime](worlds.md)). `-f`/`--follow` keeps it syncing; `--dir <PATH>` makes the replica persistent; `--timeout-secs <N>` defaults to `30`, with range `0..=86400`.

`id world replica <NODE> --dir <DIR> --admin-token <T>` is the operator form. `--dir` is required. `--once` runs a single pass and conflicts with `--follow`; `--source <HEX>` pins a source; `--lease <FILE>` takes a lease file. Its `--world <NAME>` flag is the only world-command flag with a clap default: `lobby`.

World journal replication as a network service — read replicas over a signed stream (F12.13, #806) — is a design record, carried in [Design: sync, replication and FileOps](../design/sync-fileops.md). The shipped replication surface is the mirror and replica commands above, plus the log sync of the event store.

## The walk skeleton: the sync model in miniature

`id walk` is the walking skeleton for #9106, the extensible-primitives and federation programme. It is a complete, small sync system: signed records, heads, conflicts and peer sync, with demo keys. It is not feature-gated; it works in a `--no-default-features` build (CLI survey §3). Every command takes the node directory first, and one process holds a node directory at a time.

- `id walk init <DIR> --name <NAME>` — create a node directory. `--name` is required. Re-initialising keeps the records, and the recorded name must match.
- `id walk write <DIR> --as <alice|bob> --text <TEXT>` — write a record. `--as` and `--text` are required. Writers are demo keys derived from the name. `--parent <HEX>` is repeatable; omitted, the write parents on the current heads; two or more parents resolve a conflict. The command prints an `id://…` address for the record.
- `id walk read <DIR> <ADDRESS>` — read a record by its address. Reading a record the node does not hold is an error, not a fetch: the walk never retrieves on read.
- `id walk reports <DIR>` — print heads and conflict reports.
- `id walk serve <DIR>` — run until interrupted; prints a peer token of the form `<endpoint-id>@<socket-address>` (shown in backticks; the token is the two parts joined by `@`).
- `id walk sync <DIR> --peer <TOKEN>` — sync with the peer named by the token. `--peer` is required.

The walk is the proof of the sync model because it isolates the model's moving parts at demo scale. Two node directories diverge by writing under different demo keys, converge by `walk sync`, and surface their divergence as heads and conflict reports rather than hiding it. Records move only during sync, never during read — the same rule the full system states as ingest outcomes and writer-set gating, reduced to six commands. The full command reference is in [Gallery, match-audit and walk](../reference/cli-gallery-walk.md).
