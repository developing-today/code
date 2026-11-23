---
created: 2026-11-23
last-modified: 2026-11-23
revision: 1
---

# Living documents

This folder holds programme-level **living documents**: maintained references that no single package owns — catalogues and datasets kept current by recurring jobs and lanes. For the full taxonomy (dated records vs living documents, and the framing rule for ecosystem/comparative documents), see the [documentation map](../README.md#dated-records-vs-living-documents).

## The header contract

Every living document in this folder carries, at the top:

```
---
created: YYYY-MM-DD
last-modified: YYYY-MM-DD
revision: N
---
```

and a `## Change log` section. An edit to a living document, in the same PR:

1. bumps `revision`,
2. updates `last-modified`,
3. appends a change-log line (`- rN (YYYY-MM-DD): what changed.`).

Data files that are appended mechanically (e.g. dataset tables) may live beside a headed README instead of carrying a header themselves; the README's change log then covers the folder.

## Index

- [`flag-observations/`](flag-observations/README.md) — the execution-risk flag dataset: schema, standing practice, and the observation log lanes append to.
- [`iroh/`](iroh/) — iroh ecosystem catalogues maintained by the weekly iroh digest (blog catalogue; talks and announcements).
- [`roc/`](roc/) — Roc ecosystem catalogues maintained by the weekly Roc digest and media sweep (talks and announcements).

## Change log

- r1 (2026-11-23): Folder created under the DOCS-MIGRATION programme (#5983) with the header contract and the first living documents (flag-observations; iroh and roc catalogues follow in their own PRs).
