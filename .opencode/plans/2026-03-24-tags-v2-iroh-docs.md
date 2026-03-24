# Tags v2: iroh-docs Metadata with Tuple-Encoded Keys & Real-time Web Integration

## Summary

Replace the current single-blob `MetaDoc` system (`src/tags.rs`) with an **iroh-docs** backed metadata store. Implement **FoundationDB-style tuple encoding** for binary keys with prefix-queryable structure at any depth. Wire real-time tag updates through a dedicated **WebSocket** endpoint to the web UI. Introduce a **multi-namespace** architecture with a system namespace and per-file namespaces.

**Three layers:**
1. **Tuple encoding module** — binary key encoding with prefix queries at any depth
2. **iroh-docs metadata layer** — storage, sync, namespaces, API
3. **WebSocket & web UI** — display, CRUD, real-time updates

---

## Design Decisions

### Tuple encoding over postcard/msgpack/cbor

**Problem:** Keys are binary and contain complex data (strings, integers, arrays). Need prefix queries at any depth — including mid-field (e.g., "all strings starting with 'read'").

**Why not postcard:** Length-prefixed strings prevent mid-field prefix queries. `serialize("two") = [03, t, w, o]` and `serialize("twelve") = [06, t, w, e, l, v, e]` don't share a byte prefix despite sharing character prefix "tw".

**Why not msgpack/cbor:** Same length-prefix problem for strings. Array wrappers break field-boundary prefix queries. Non-canonical encoding risks (msgpack).

**Solution:** FoundationDB-style tuple layer encoding where string bytes appear RAW (only `\0` escaped as `\0\xFF`, terminated by `\0`). Mid-field prefix = just omit the terminator.

**Integers:** Big-endian encoding with byte count in type tag. Big-endian is required for lexicographic sort order to match numeric order (little-endian native on x86/ARM but byte-swap is 1 cycle via `u64::to_be_bytes()`).

### iroh-docs over single-blob MetaDoc

| Single-blob MetaDoc (current)       | iroh-docs                                  |
| ------------------------------------ | ------------------------------------------ |
| Load-modify-save bottleneck          | Per-entry writes, no contention            |
| No sync                              | Built-in sync via gossip between peers     |
| No provenance                        | Author signatures (ed25519) on every entry |
| No entry timestamps                  | Automatic timestamp on every entry         |
| No change notifications              | `subscribe()` → real-time LiveEvent stream |
| JSON parse entire doc for any query  | Prefix queries on binary keys              |

### Multi-namespace architecture

- **System namespace** — default read/write target, associated with this iroh node. Holds cross-cutting metadata (tags for all files). One gossip topic, one sync boundary.
- **Per-file namespaces** — each friendly name gets its own iroh-doc. Enables per-file sync granularity, per-file sharing (DocTicket), per-file access control.
- Namespace mapping stored in system namespace: key `("_ns", filename)` → namespace_id bytes.

### Separate WebSocket for tags (not extending collab)

- Collab WebSocket is **per-document** — tied to ProseMirror content collaboration
- Tags are **cross-document metadata** — different lifecycle, different consumers
- Main page needs tag updates for ALL files simultaneously
- Clean separation of concerns

### Persistent store

- `Docs::persistent(path)` with `fs-store` feature — metadata survives restarts
- Store path: `{store_path}/docs/`

---

## Layer 1: Tuple Encoding Module

### Type tags and sort order

```
Tag(s)      Type         Payload
0x00        Null         none (escaped to 0x00 0xFF inside nested tuples)
0x01        Bytes        raw + escape(0x00→0x00 0xFF) + 0x00 terminator
0x02        String       same as bytes, typed as UTF-8
0x05        Tuple/Array  recursive elements + 0x00 end
0x06        False        none
0x07        True         none
0x0C–0x13   Neg int      8–1 bytes ones-complement big-endian
0x14        Zero         none
0x15–0x1C   Pos int      1–8 bytes big-endian
0x21        Float64      8 bytes sort-adjusted IEEE 754
```

Sort order: `null < bytes < string < tuple < false < true < -big…-small < 0 < small…big < float`

### Encoding examples

**Strings** — bytes appear raw, only `\0` escaped:
```
"readme"   → [02, r, e, a, d, m, e, 00]
"read"     → [02, r, e, a, d, 00]
"re\0ad"   → [02, r, e, 00, FF, a, d, 00]    // \0 escaped as 0x00 0xFF
""         → [02, 00]                          // just type tag + terminator
```

**Booleans** — zero-byte payload:
```
false      → [06]
true       → [07]
```

**Integers** — big-endian, byte count in type tag:
```
0          → [14]
1          → [15, 01]
255        → [15, FF]
256        → [16, 01, 00]
-1         → [13, FE]                         // ones-complement of 0x01
-256       → [12, FE, FF]                     // ones-complement of [0x01, 0x00]
-1000      → [12, FC, 17]                     // ones-complement of [0x03, 0xE8]
i64::MIN   → [0C, 7F, FF, FF, FF, FF, FF, FF, FF]
i64::MAX   → [1C, 7F, FF, FF, FF, FF, FF, FF, FF]
```

**Float64** — sort-adjusted IEEE 754:
```
Positive: flip sign bit only    → 0.0 < 1.0 < inf ✓
Negative: flip ALL bits         → -inf < -1.0 < -0.0 ✓
-0.0 canonicalized to +0.0
NaN rejected (encode returns error)
```

**Nested tuples/arrays:**
```
("a", [1, 2]) → [02, 61, 00,  05, 15, 01, 15, 02, 00]
                  ^^^^^^^^     ^^  ^^^^^^  ^^^^^^  ^^
                  "a"          [   1       2       ]

[null, 1]     → [05, 00, FF, 15, 01, 00]
                  ^^  ^^^^^  ^^^^^^  ^^
                  [   null   1       ]
                  (null escaped as 0x00 0xFF inside nested tuple)
```

**Structs** — positional encoding (field names from schema, not encoded):
```
{a: 1, b: [0,1,2,3], c: null} →
  [15, 01,  05, 14, 15, 01, 15, 02, 15, 03, 00,  00]
   ^^^^^^   ^^  ^^  ^^^^^^  ^^^^^^  ^^^^^^  ^^   ^^
   a=1      [   0   1       2       3       ]    null
```

### Prefix query patterns

```
// Field-boundary prefix (exact field match, then any suffix):
all tags for "README.md":
  prefix = [02, R,E,A,D,M,E,.,m,d, 00]      // complete string + terminator

// Mid-field prefix (partial string match):
all files starting with "READ":
  prefix = [02, R,E,A,D]                      // string tag + partial, NO terminator

// Multi-field prefix:
all "label" tags for "README.md":
  prefix = [02, R,E,A,D,M,E,.,m,d, 00,  02, l,a,b,e,l, 00]

// Mid-array prefix (inside nested array, before end marker):
a=1, b starts with [0, 1]:
  prefix = [15, 01,  05, 14, 15, 01]         // int(1) + array_start + int(0) + int(1)
  matches {a:1, b:[0,1,2,3]} and {a:1, b:[0,1,5]}
```

### Null disambiguation inside nested tuples

Inside a nested tuple, `0x00` is ambiguous (null element vs end-of-tuple):
- `0x00 0xFF` = null element (escaped)
- bare `0x00` = end of nested tuple

This uses the same escape mechanism as strings/bytes. The decoder tracks nesting depth.

### Rust API

```rust
/// A decoded tuple element (owned).
pub enum TupleValue {
    Null,
    Bytes(Vec<u8>),
    String(String),
    Tuple(Vec<TupleValue>),
    Bool(bool),
    Int(i64),
    Float(f64),
}

/// Builder for encoding tuple keys.
pub struct TupleEncoder { buf: Vec<u8>, nested: bool }

impl TupleEncoder {
    pub fn new() -> Self;

    // Full field encoding (type tag + payload + terminator)
    pub fn null(&mut self) -> &mut Self;
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self;
    pub fn string(&mut self, s: &str) -> &mut Self;
    pub fn bool(&mut self, v: bool) -> &mut Self;
    pub fn int(&mut self, v: i64) -> &mut Self;
    pub fn float(&mut self, v: f64) -> Result<&mut Self>;  // rejects NaN
    pub fn array(&mut self, f: impl FnOnce(&mut Self)) -> &mut Self;

    // Prefix encoding (no terminator/end marker — for "starts with" queries)
    pub fn string_prefix(&mut self, s: &str) -> &mut Self;
    pub fn bytes_prefix(&mut self, b: &[u8]) -> &mut Self;
    pub fn array_prefix(&mut self, f: impl FnOnce(&mut Self)) -> &mut Self;

    pub fn build(self) -> Vec<u8>;
}

/// Decode all elements from tuple-encoded bytes.
pub fn decode(bytes: &[u8]) -> Result<Vec<TupleValue>>;
```

Accessor methods on `TupleValue`: `as_str()`, `as_bytes()`, `as_int()`, `as_float()`, `as_bool()`, `as_tuple()`, `is_null()`.

### Implementation scope

~500-600 lines of Rust in `src/tuple.rs`. Zero new dependencies. Thorough tests:
- Encode/decode roundtrip for all types (null, bytes, string, bool, int, float, tuple)
- Integer boundaries (0, 1, -1, 255, 256, i64::MIN, i64::MAX)
- String edge cases (empty, contains `\0`, unicode)
- Float edge cases (+0.0, -0.0 canonical, ±inf, NaN rejected)
- Nested tuples (empty, with nulls, deeply nested)
- Sort order within types and across type boundaries
- Prefix query correctness (field-boundary, mid-field, mid-array)
- Struct encoding ({a: 1, b: [0,1,2,3], c: null})

### Files

| File | Change |
|------|--------|
| `src/tuple.rs` | **New** — tuple encoding module |
| `src/lib.rs` | Add `pub mod tuple;` |

---

## Layer 2: iroh-docs Metadata

### 2.1 Dependency

**Cargo.toml:**
```toml
iroh-docs = { version = "0.97", features = ["fs-store"] }
```

### 2.2 Initialize iroh-docs in `src/commands/serve.rs`

**Problem:** `Docs::spawn()` requires `Gossip`, but gossip is only created when `!no_gossip`.

**Solution:** Always create Gossip. Only register `GOSSIP_ALPN` on router when gossip enabled.

```rust
// Always (needed for Docs even in no-gossip mode)
let gossip = Gossip::builder().spawn(endpoint.clone()).await?;
let docs = Docs::persistent(&store_path.join("docs"))
    .spawn(endpoint.clone(), store.clone(), gossip.clone())
    .await?;
let author_id = docs.author_default().await?;
let meta_doc = get_or_create_meta_doc(&docs, &store_path).await?;

// Register ALPN
router = router.accept(iroh_docs::net::ALPN, docs.clone());

// Only when gossip enabled
if !no_gossip {
    router = router.accept(iroh_gossip::net::GOSSIP_ALPN, gossip.clone());
    // ... existing gossip logic ...
}
```

**`get_or_create_meta_doc`:**
- Check `{store_path}/meta-doc-id` file for stored NamespaceId
- If found: `docs.open(id)` — reuse existing
- If not found: `docs.create()`, save NamespaceId to file
- Run migration check (2.5)

### 2.3 Update AppState (`src/web/mod.rs`)

```rust
pub struct AppState {
    pub store: Store,                     // existing — iroh-blobs
    pub meta_doc: Doc,                    // NEW — system metadata document
    pub author_id: AuthorId,              // NEW — this node's author for writes
    pub tag_broadcast: Arc<TagBroadcast>, // NEW — real-time tag event broadcaster
    pub collab: Arc<CollabState>,
    pub assets: AssetUrls,
    pub peers: Option<PeerDiscovery>,
    pub node_id: String,
}
```

### 2.4 Rewrite `src/tags.rs` — iroh-docs backed API

**Key schema using tuple encoding:**

```rust
// Single-value tag: subject + key → value in entry bytes
key = TupleEncoder::new().string("README.md").string("created").build()
val = b"1711152000"

// Multi-value tag: subject + key + value → empty entry
key = TupleEncoder::new().string("README.md").string("label").string("rust").build()
val = b""

// Namespace mapping (system namespace only):
key = TupleEncoder::new().string("_ns").string("README.md").build()
val = namespace_id_bytes

// Complex future tag with integer:
key = TupleEncoder::new().string("README.md").string("version").int(3).build()
val = b""
```

**API:**

```rust
/// A resolved tag entry with full metadata from iroh-docs.
pub struct TagEntry {
    pub subject: String,
    pub key: String,
    pub value: String,
    pub author: AuthorId,
    pub timestamp: u64,
}

// === Core CRUD ===
pub async fn set_tag(doc: &Doc, author: AuthorId, subject: &str, key: &str, value: &str) -> Result<()>;
pub async fn get_tags(doc: &Doc, subject: &str) -> Result<Vec<TagEntry>>;
pub async fn get_tag(doc: &Doc, subject: &str, key: &str) -> Result<Option<TagEntry>>;
pub async fn remove_tag(doc: &Doc, author: AuthorId, subject: &str, key: &str) -> Result<()>;
pub async fn remove_all_tags(doc: &Doc, author: AuthorId, subject: &str) -> Result<()>;

// === Multi-value ===
pub async fn add_multi_tag(doc: &Doc, author: AuthorId, subject: &str, key: &str, value: &str) -> Result<()>;
pub async fn get_multi_tags(doc: &Doc, subject: &str, key: &str) -> Result<Vec<TagEntry>>;
pub async fn remove_multi_tag(doc: &Doc, author: AuthorId, subject: &str, key: &str, value: &str) -> Result<()>;

// === Convenience ===
pub async fn set_created(doc: &Doc, author: AuthorId, name: &str) -> Result<()>;
pub async fn set_modified(doc: &Doc, author: AuthorId, name: &str) -> Result<()>;
pub async fn transfer_tags(doc: &Doc, author: AuthorId, old_name: &str, new_name: &str) -> Result<()>;
pub async fn add_archive_tag(doc: &Doc, author: AuthorId, name: &str, archive_name: &str) -> Result<()>;
pub async fn get_created_time(doc: &Doc, name: &str) -> Result<Option<u64>>;
pub async fn get_modified_time(doc: &Doc, name: &str) -> Result<Option<u64>>;

// === Query ===
pub async fn find_by_tag(doc: &Doc, key: &str, value: &str) -> Result<Vec<String>>;
pub async fn list_all_tags(doc: &Doc) -> Result<Vec<TagEntry>>;
```

**Implementation notes:**
- `set_tag` → `doc.set_bytes(author, tuple(subject, key), value.as_bytes())`
- `get_tags` → `doc.get_many(Query::single_latest_per_key().key_prefix(tuple_prefix(subject)))` then decode
- `remove_tag` → `doc.del(author, tuple(subject, key))` (inserts empty entry)
- `transfer_tags` → read all for old subject, write under new, delete old
- Uses `Query::single_latest_per_key()` for last-write-wins conflict resolution

### 2.5 Migrate existing MetaDoc data

On first startup with new system:
1. Check if old `.meta` blob tag exists: `store.tags().get(Tag::from(".meta"))`
2. If found: read blob → deserialize JSON `MetaDoc`
3. For each `MetaTag`: write as iroh-docs entry using tuple key encoding
4. Delete `.meta` blob tag
5. Log: "Migrated {n} metadata tags from .meta blob to iroh-docs"

### 2.6 Update integration points

All callers pass `(meta_doc, author_id)` from AppState:

| File | Function | Change |
|------|----------|--------|
| `src/web/routes.rs` | `get_file_list()` | Use `get_tags(doc, name)` per file |
| `src/web/routes.rs` | `save_handler` | `set_modified(doc, author, name)` |
| `src/web/routes.rs` | `new_file_handler` | `set_created` + `set_modified` |
| `src/web/routes.rs` | `rename_handler` | `transfer_tags` + `add_archive_tag` |
| `src/protocol.rs` | rename handler | `transfer_tags(doc, author, old, new)` |
| `src/commands/repl.rs` | rename command | `transfer_tags(doc, author, old, new)` |

Remove all `load_meta()`/`save_meta()` calls.

### 2.7 Namespace management

**System namespace:**
- Created once on first run, NamespaceId persisted to disk
- All tag operations default to this namespace
- Keys: `tuple(subject, tag_key, [tag_value])`

**Per-file namespaces:**
- Created lazily on first request (or when sharing a specific file)
- Mapping stored in system namespace: `tuple("_ns", filename)` → namespace_id bytes
- Within file namespace: keys are `tuple(tag_key, [tag_value])` (no subject — namespace IS the subject)
- Enables: `meta_doc.share(ShareMode::Read, addr_options)` for per-file sharing

```rust
pub async fn get_or_create_file_namespace(
    docs: &DocsApi,
    system_doc: &Doc,
    author: AuthorId,
    filename: &str,
) -> Result<Doc>;

pub async fn get_file_namespace(
    docs: &DocsApi,
    system_doc: &Doc,
    filename: &str,
) -> Result<Option<Doc>>;
```

### 2.8 Subscribe to LiveEvents

Background task on startup:

```rust
fn spawn_tag_event_listener(doc: Doc, broadcast: Arc<TagBroadcast>) {
    tokio::spawn(async move {
        let mut events = doc.subscribe().await?;
        while let Some(Ok(event)) = events.next().await {
            match event {
                LiveEvent::InsertLocal { entry } | LiveEvent::InsertRemote { entry, .. } => {
                    let key_bytes = entry.key();
                    if let Ok(fields) = tuple::decode(key_bytes) {
                        let is_delete = entry.content_len() == 0;
                        // Parse fields, construct TagEvent, broadcast
                        broadcast.emit(tag_event);
                    }
                }
                _ => {}
            }
        }
    });
}
```

### Layer 2 files

| File | Change |
|------|--------|
| `Cargo.toml` | Add `iroh-docs = { version = "0.97", features = ["fs-store"] }` |
| `src/tags.rs` | **Complete rewrite** — tuple keys + iroh-docs CRUD + migration |
| `src/commands/serve.rs` | Init Gossip always, init Docs, create meta doc, spawn listener |
| `src/web/mod.rs` | Add `meta_doc`, `author_id`, `tag_broadcast` to AppState |
| `src/web/routes.rs` | Update all tag callers to new async API |
| `src/protocol.rs` | Update rename tag transfer |
| `src/commands/repl.rs` | Update rename tag transfer |

---

## Layer 3: WebSocket & Web UI

### 3.1 Tag CRUD REST API

```
GET    /api/tags                → list all tags (?subject=, ?key= filters)
GET    /api/tags/:subject       → list tags for a file
POST   /api/tags                → add tag { subject, key, value? }
DELETE /api/tags                → remove tag { subject, key, value? }
```

JSON response:
```json
{
  "tags": [
    { "subject": "README.md", "key": "category", "value": "docs", "author": "abc123", "timestamp": 1711152000 }
  ]
}
```

### 3.2 Tag WebSocket endpoint

```
/ws/tags               → global tag updates (main page)
/ws/tags/:subject      → tag updates for specific file (editor page)
```

Wire protocol (MessagePack for consistency with collab):
```
[0, tags[]]                                → TagInit: full state on connect
[1, subject, key, value, author, timestamp] → TagSet: server→client
[2, subject, key]                          → TagRemove: server→client
[3, subject, key, value?]                  → TagSetRequest: client→server
[4, subject, key]                          → TagDeleteRequest: client→server
```

### 3.3 Tag broadcast infrastructure

New module `src/web/tags_ws.rs`:

```rust
pub struct TagBroadcast {
    global: broadcast::Sender<TagEvent>,
    subjects: RwLock<HashMap<String, broadcast::Sender<TagEvent>>>,
}

pub enum TagEvent {
    Set { subject: String, key: String, value: String, author: String, timestamp: u64 },
    Remove { subject: String, key: String },
}

impl TagBroadcast {
    pub fn new() -> Self;
    pub fn emit(&self, event: TagEvent);
    pub fn subscribe_global(&self) -> broadcast::Receiver<TagEvent>;
    pub fn subscribe_subject(&self, subject: &str) -> broadcast::Receiver<TagEvent>;
}
```

### 3.4 Main page tag display

**FileInfo update:**
```rust
pub struct FileInfo {
    // existing...
    pub tags: Vec<TagEntry>,  // all user-visible tags
}
```

**Template changes:**
- Tag pills/badges beside each filename
- Color-coded by tag key
- System tags (created, modified, archived) shown as they are
- User tags shown as clickable pills
- Search bar gains `tag:key=value` filter syntax

**Real-time:**
- Connect `/ws/tags` on page load
- On TagSet/TagRemove: update file row's tag display in-place

### 3.5 Editor page tag display + CRUD

- Tag section in header or sidebar
- All tags with author attribution
- "Add tag" form: key + optional value
- "x" button to remove tag pills
- Connect `/ws/tags/{filename}` on load
- Send TagSetRequest/TagDeleteRequest via WebSocket

### 3.6 Client-side JavaScript

In `web/src/tags.ts` (new module):

```typescript
interface TagEntry { subject: string; key: string; value: string; author: string; timestamp: number; }
type TagEvent = { type: 'set'; subject: string; key: string; value: string; author: string; timestamp: number }
              | { type: 'remove'; subject: string; key: string };

function initTagsWs(url: string, onEvent: (event: TagEvent) => void): WebSocket;
async function addTag(subject: string, key: string, value?: string): Promise<void>;
async function removeTag(subject: string, key: string): Promise<void>;
async function getTags(subject?: string): Promise<TagEntry[]>;
function renderTagPills(container: Element, tags: TagEntry[]): void;
function renderTagEditor(container: Element, subject: string): void;
```

### Layer 3 files

| File | Change |
|------|--------|
| `src/web/routes.rs` | Add `/api/tags` endpoints, `/ws/tags` routes |
| `src/web/tags_ws.rs` | **New** — TagBroadcast, WebSocket handler |
| `src/web/mod.rs` | Add `mod tags_ws` |
| `src/web/templates.rs` | Tag pills in file list, tag editor in editor page |
| `web/src/tags.ts` | **New** — WebSocket client, REST, UI |
| `web/src/main.ts` | Import and init tags module |
| `web/styles/terminal.css` | Tag pill/badge styling |

---

## Implementation Order

1. `src/tuple.rs` — tuple encoding (standalone, testable independently)
2. `Cargo.toml` + `src/commands/serve.rs` — iroh-docs init
3. `src/tags.rs` — rewrite with iroh-docs + tuple keys
4. Integration points — update all callers
5. Migration — MetaDoc → iroh-docs one-time migration
6. `src/web/tags_ws.rs` — broadcast infrastructure
7. REST API endpoints
8. WebSocket endpoints
9. Template updates (tag pills)
10. Client-side JS (tags.ts)

Steps 1-5 are Phase 1 (iroh level). Steps 6-10 are Phase 2 (web UI).

---

## Open Questions

1. **Per-file namespace creation policy:** Eager (create on file creation) or lazy (create on first share/specific request)?

2. **Tag sharing between peers:** When a peer connects, how do they learn the system namespace's NamespaceId? Options: include in gossip PeerAnnouncement, exchange during ALPN handshake, manual DocTicket.

3. **Delete semantics:** `doc.del()` inserts empty entries that propagate via sync. Alternative: use sentinel value instead of `del()` to avoid uncontrollable delete propagation. Decision needed.

4. **Conflict resolution:** `single_latest_per_key()` = last-write-wins. Should UI surface multi-author conflicts?

5. **Internal tag filtering:** `created`, `modified`, `archived`, `_ns` are system tags. Show with different styling vs hide from user tag CRUD?
