# Offline provider-catalog audit snapshots

`provider-catalog-sync.mjs` normally reads provider proxy `/v1/models` lists
from laptop-only ports. A detached audit cycle cannot reach those ports. On a
machine where the proxies are running, capture a portable snapshot:

```sh
node bin/snapshot-provider-catalogs.mjs --output-dir /path/to/catalog-snapshot
```

The command writes one `<port>.json` file per reachable proxy, a
`models.dev.json` cross-check, and `manifest.json` with the UTC capture time
and each source's status. Proxy responses are reduced to model IDs. An
unavailable proxy is recorded in the manifest and left without a port file;
the sync tool will mark its provider not-enumerable rather than report an
empty catalog. `models.dev` is required for a complete portable cross-check;
the command exits nonzero if that fetch fails.

Copy or otherwise hand off the entire directory to the detached cycle, then
run the normal dry-run using only local files:

```sh
node bin/provider-catalog-sync.mjs \
  --source-dir /path/to/catalog-snapshot \
  --models-dev /path/to/catalog-snapshot/models.dev.json \
  --json
```

The report distinguishes validated additions, catalog-only unverified
additions, vanishings (flag only), and unverifiable/unavailable providers.
Include `manifest.json`'s `captured_at` in the audit entry: the result is a
diff against that snapshot time, not a claim about catalogs after capture.
Exit status 3 means validated additions were found; 0 means none were found;
2 means the audit could not run.
