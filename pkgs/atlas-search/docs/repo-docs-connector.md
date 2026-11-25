# Repo docs connector (#8626)

Offline ingestion into the existing `repo-docs` corpus. Provision with
`atlas-stage provision-corpora --registry registry.db --dir staging`, then:

```sh
atlas-stage ingest-repo-docs --db staging/repo-docs.db --roots roots.json --agent muse
```

`roots.json` is a strict JSON array. Configure only checkouts actually present:

```json
[
  {"repo":"developing-today/code","root":"/workspace/code"},
  {"repo":"developing-today-forks/opencode","root":"/workspace/opencode"},
  {"repo":"developing-today-forks/openchamber","root":"/workspace/openchamber"},
  {"repo":"dezren39/darwin-nix","root":"/workspace/darwin-nix"}
]
```

Unknown IDs/fields, duplicate IDs, overlapping roots, relative/traversal paths,
missing directories, unreadable/non-UTF-8 documents, and symlinks in selected
trees are errors. Roots and their ancestors must be real directories. No Git
commands, network fetching, or external content discovery occurs. Checkouts
must remain quiescent during a scan; this is not a hostile-filesystem sandbox
and concurrent path replacement is not supported.

The deterministic path policy selects `.md` (case-insensitive) files under
top-level `doc/` and `docs/`, root `README.md`, and `doc/`, `docs/` and READMEs
under `pkgs/` or `packages/`. Hidden directories, `node_modules`, `target`, and
`vendor` are excluded. Package trees are inspected but other package source
Markdown is not ingested. There is deliberately no arbitrary include glob.

Source identity is `repo:relative/path`, independent of checkout location and
commit. Each prose item carries corpus, optional agent (JSON null if unknown),
source_ref, filesystem modification time as RFC3339 `updated_at`, sensitivity
(`--sensitivity`, default internal), and `zone=fleet`. A rescan compares text
and provenance: unchanged rows/chunks remain byte-for-byte untouched. An edit
updates the existing item and replaces only that document's chunks; the scan
is one SQLite transaction, including rollback on any chunk write failure.
Deleted/renamed files are not pruned in this slice; use a fresh corpus rebuild
for deletion reconciliation. No embedding occurs during ingestion.

Promotion uses the existing registry/model gate and internal fan-out. That is
not acceptance evidence for the MCP search surface (#8625 remains separate).
Other connector families and all-corpora MCP queryability remain outside this
repo-docs slice of #8626.

Targeted oracle suite (dispatch remotely, not in a sandbox):

```sh
cargo test --manifest-path pkgs/atlas-search/Cargo.toml -p atlas-core --test repo_docs --test memory --test stage
```
