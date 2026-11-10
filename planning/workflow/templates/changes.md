# Change log template

`init` writes a package's first `changes.md`. It is the one shared package file every writer adds to directly: one line at the end, in the same commit as the change it records. `.gitattributes` gives it `merge=union`, so parallel branches merge without conflicts; that only works if each line stands alone. Never edit or remove a line; correct one with a new line.

```markdown
# Changes

[Main reference](main.md)

- <UTC> | <stage> | <status> | <IDs>: <what changed>, by <writer> (<path, PR or commit>)
```

- `<stage>` is `S0` to `S8`; `<status>` is the item status, disposition or run status the change set ([records](../records.md)).
- `item add` and `item set-status` append their own line; add one by hand only for changes no command records.
- Example: `- 2026-10-21T10:00:00Z | S2 | clarified | C001.2, C001.2.1: split the nuance into its own item, by c001-research (#512)`.
