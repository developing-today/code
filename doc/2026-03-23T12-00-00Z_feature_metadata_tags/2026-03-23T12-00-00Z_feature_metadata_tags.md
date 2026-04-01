# Metadata Tag System

See [original plan](../../.opencode/plans/) (inline design, no separate plan file).

## Intent

The existing iroh-blobs tag system (`name -> content_hash`) serves as file storage. But richer metadata is needed:

- **Created/modified dates** that survive content changes
- **Arbitrary key/value tags** on files (categories, labels, notes)
- **Non-unique keys** (multiple tags with same key, e.g. `tag=rust`, `tag=cli`)
- **Key-only tags** (flags like `pinned`, `favorite`)
- **Linking** - a tag can point to a content hash or another filename
- **Tag timestamps** - every tag records when it was created and last modified

## Design

### Data Model

```rust
/// The metadata document stored as a blob in iroh-blobs.
/// Referenced by the special iroh-blobs tag `.meta`.
struct MetaDoc {
    version: u32,
    tags: Vec<MetaTag>,
}

/// A single metadata tag.
struct MetaTag {
    /// Subject this tag describes (usually a filename like "README.md")
    subject: String,
    /// Tag key (e.g., "created", "category", "author")
    key: String,
    /// Optional value. None for key-only tags like "pinned"
    value: Option<String>,
    /// Optional link to another entity
    link: Option<MetaLink>,
    /// When this tag was created (unix seconds)
    created_at: u64,
    /// When this tag was last modified (unix seconds)
    modified_at: u64,
}

/// A link target - points to a hash or a filename.
enum MetaLink {
    /// Points to a content hash (hex string)
    Hash(String),
    /// Points to a filename / tag name
    Name(String),
}
```

### Storage Mechanism

Metadata is stored as a JSON blob in the iroh-blobs store, referenced by the
reserved tag name `.meta`. This approach:

- **Syncs via iroh** - the metadata blob is just another content-addressed blob
- **Atomic updates** - load, modify, save cycle with single tag update
- **No external deps** - uses existing serde_json (already a dependency)
- **No schema migration** - version field allows future evolution

Update cycle:

1. Load: get hash from `.meta` tag -> read blob -> deserialize JSON
2. Modify: add/remove/update tags in memory
3. Save: serialize JSON -> add as blob -> update `.meta` tag to new hash

If `.meta` tag doesn't exist, start with empty `MetaDoc { version: 1, tags: [] }`.

### API (`src/web/tags.rs`)

Core I/O:

- `load_meta(store) -> MetaDoc` - loads from `.meta` tag, returns empty doc if missing
- `save_meta(store, doc) -> Result<()>` - serializes and stores

Tag CRUD:

- `add_tag(doc, subject, key, value?, link?)` - creates new tag with timestamps
- `remove_tags(doc, subject, key) -> Vec<MetaTag>` - removes all matching, returns removed
- `get_tags(doc, subject) -> Vec<&MetaTag>` - all tags for a subject
- `get_by_key(doc, subject, key) -> Vec<&MetaTag>` - tags for subject with specific key
- `set_unique_tag(doc, subject, key, value?, link?)` - removes existing tags with same subject+key, adds new one (for "created", "modified" semantics)

Convenience:

- `set_created(doc, name)` - adds unique "created" tag with current timestamp
- `set_modified(doc, name)` - adds unique "modified" tag with current timestamp
- `transfer_tags(doc, old_name, new_name)` - moves all tags from one subject to another (for rename)
- `now_unix() -> u64` - current unix timestamp helper

### Relationship to iroh-blobs Tags

```
iroh-blobs tags (file storage):
  "README.md"                    -> hash_abc123  (primary file)
  "auto-2026-03-12T06:42:30Z"   -> hash_abc123  (auto-backup)
  "README.md.archive.1711152000" -> hash_def456  (archive)
  ".meta"                        -> hash_meta01  (metadata document)

MetaDoc (inside the .meta blob):
  { subject: "README.md", key: "created",  value: "1711152000" }
  { subject: "README.md", key: "modified", value: "1711155600" }
  { subject: "README.md", key: "category", value: "documentation" }
  { subject: "README.md", key: "related",  link: Name("CHANGELOG.md") }
```

The `.meta` tag is itself an iroh-blobs tag, so it gets synced like any other content.
The `classify_tag()` function in routes.rs skips `.meta` (treats it as internal).

### Integration Points

1. **File creation** (`new_file_handler`, `save_handler` when new):
   - `set_created(doc, name)` + `set_modified(doc, name)`

2. **File save** (`save_handler`):
   - `set_modified(doc, name)`

3. **File rename** (`rename_handler`, `protocol.rs`, `repl.rs`):
   - `transfer_tags(doc, old_name, new_name)`
   - `set_modified(doc, new_name)` (rename is a modification)

4. **File list** (`get_file_list`):
   - Load MetaDoc once, look up created/modified for each file
   - Add to `FileInfo` struct for display

5. **Archive** (`archive_original_tag`):
   - Add `archived` tag with link to archive copy and operation type

### File Changes

- **New**: `src/web/tags.rs` - MetaDoc types + load/save + tag operations
- **Modified**: `src/web/mod.rs` - add `mod tags; pub use tags::*;`
- **Modified**: `src/web/routes.rs` - use tags in save/new/rename/list handlers, add `.meta` to classify_tag skip list
- **Modified**: `src/protocol.rs` - use tags in rename handler
- **Modified**: `src/commands/repl.rs` - use tags in rename

## References

- iroh-blobs tag API: `store.tags().get/set/delete` (see `src/store.rs`)
- Existing file list UI feature: `docs/2026-03-23T00:00:00Z_feature_file_list_ui/`
