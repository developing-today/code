# d1-gateway

A Cloudflare Worker that answers the Turso/libSQL remote protocol
(Hrana JSON pipelines, `/v2/pipeline` and `/v3/pipeline`) from Cloudflare
D1, so a Turso client moves to D1 by changing its URL:

    https://d1-gateway.developing-today.workers.dev/db/<database>

Clients append `/v2/pipeline` as they do for Turso and send their existing
`turso.<database>` token as the bearer; the Worker accepts a token when its
SHA-256 is listed for that database in `TOKEN_SHA256` (wrangler.jsonc).
Workers in the account can instead bind the `Gateway` entrypoint and call
`env.D1GW.pipeline("<database>", body)`.

Semantics D1 imposes (see src/hrana.js): no batons, so nothing outlives one
request (no TEMP tables across requests); `BEGIN ... COMMIT` inside one
pipeline, and Hrana batches shaped BEGIN / statements / conditional COMMIT /
ROLLBACK, run as one atomic D1 batch; at most 100 bound parameters per
statement and 100 KB per statement (D1 limits).

Deploy: `.github/workflows/d1-gateway-deploy.yml` on every push to main that
touches this directory (wrangler `versions upload`, then `versions deploy`);
rollback by running the workflow with a version id. Locally:
`nix run .#d1-gateway-deploy` with CLOUDFLARE_API_TOKEN and
CLOUDFLARE_ACCOUNT_ID set.

Copying a Turso database in: `infra/fleet-queue-backup/bin/export_to_d1.py
--source-url https://<db>-dezren39.aws-us-east-1.turso.io --d1-name <db>`
with that database's token in TURSO_QUEUE_TOKEN.
