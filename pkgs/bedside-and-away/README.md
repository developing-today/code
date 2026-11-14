# Bedside & Away

A private shopping, research and decision workspace with separate personal/shared/open stores, stable item IDs, versioned requests and audit history. The app lives in `pkgs/bedside-and-away`; [#9359](https://github.com/developing-today/code/issues/9359) is the umbrella spec and [#9361](https://github.com/developing-today/code/pull/9361) is the initial design. See `design/` for contracts and explicit coverage limits.

## Run locally

From this directory with Node 24 or later:

```sh
npm test
node tests/local-server.mjs
```

The second command starts an ephemeral loopback server and prints its URL. It uses a synthetic user and in-memory SQLite. It must not be exposed or pointed at production. In the UI, use **Set up my stores** to create sample data. The original private cart is not in the fixtures or repository. Product evidence is a historical observation, not a verified checkout quote.

## Production topology

The private Tailscale Serve listener feeds a loopback gateway that verifies allowed identities and signs exact API requests. Cloudflare hosts the Worker; Turso is the dedicated authoritative database. The older private Sites/D1 instance is separate, not synchronized. No Atlas or Snowflake integration. Read [deployment and rollback](design/release.md) and [external adapter](external/README.md) before releasing anything. Use native `cf`; no Wrangler configuration.

## Scope boundaries

Manual steering creates a versioned request and separate reviewed proposal; it does not contact Archer automatically. Market research briefs and product/receipt intake are user-reviewed, not automated browsing or receipt OCR. Source records remain private unless their store access allows disclosure. The browser keeps a random installation ID, not a fingerprint or an authentication token. Live guest linking, external sign-ins, original cart recovery and live provider refresh need their own integration evidence. Do not call this build fully accepted or deployed solely because tests pass.
