# Grist Choice options: the supported, scripted update path (#1287)

**Verified against Grist OSS v1.7.21** — the exact image pinned by
digest in `compose.yml` (`gristlabs/grist-oss@sha256:d761de16…`) —
on a disposable document, 2026-11-01 (queue task 5000). Re-verify
after any Grist version bump: the ACL behavior below is an observed
property of this version, not a documented upstream guarantee.

## TL;DR

Use `update-choice-options.py` in this directory. Do **not** use
`/apply` with `UpdateColumn`, and never edit the document SQLite.

```bash
cd pm-stack/grist
GRIST_API_KEY=<key> python3 update-choice-options.py \
    --doc <docId> --table Roadmap --column Area --add <new-choice>
```

`GRIST_API_KEY` comes from SOPS (`grist_api_key`, the dedicated admin
account's key) exactly as in `seed.sh`; the script reads it from the
environment only. Additions are idempotent, existing choices and all
other `widgetOptions` (e.g. `choiceOptions` colors) are preserved,
and the result is verified by read-back before the script exits 0.

## What was probed (disposable doc, pinned image)

| Call | Caller / rules | Result |
| --- | --- | --- |
| `GET …/tables/Roadmap/columns` | doc owner key | 200; `widgetOptions` is a JSON **string** |
| `PATCH …/tables/Roadmap/columns/{colId}` | owner | **404** — this per-column route does not exist in v1.7.21 (checked against the image's own `DocApi.js`) |
| `PATCH …/tables/Roadmap/columns` with `{"columns":[{"id":"Area","fields":{"widgetOptions":"{…}"}}]}` | owner | 200; choices extended, `choiceOptions` preserved, read-back matches |
| `POST …/apply` `[["UpdateColumn",…]]` | same key, doc rule set to `+CRUD` (data access, **no** `S`/schemaEdit — the roadmap doc's shape) | **500 `Blocked by access rules: 'UpdateColumn' actions are not controlled`** — the exact production error from #1287, reproduced |
| records `POST` (data write) | same key, `+CRUD` | 200 — data access is unaffected, as in production |
| `PATCH …/tables/Roadmap/columns` (collection route) | same key, `+CRUD` | **200 — applied and read back** under the same rules that block `/apply` |
| `update-choice-options.py --add ops` ×2, then `--dry-run` | same key, `+CRUD` | add + read-back verified; second run a no-op; dry run sends nothing |

So the REST columns route is a supported, authenticated, non-SQLite
path that works under the roadmap ACL — issue #1287 end state (a).
Whether the `/apply`-vs-REST ACL difference is an intentional
upstream distinction or an enforcement gap in v1.7.21 is not
documented upstream; treat it as version-specific behavior and
re-run this probe after upgrading Grist, before relying on it.

## Notes

- The script's merge logic is unit-tested:
  `python3 -m unittest test_update_choice_options` (stdlib only).
- Removing choices is deliberately not supported: dropping a choice
  that rows still use is a data decision, not an options edit.
- Production use still takes the usual care: this changes the live
  roadmap doc's schema metadata, so run it deliberately (a `--dry-run`
  first), not from a cron.
