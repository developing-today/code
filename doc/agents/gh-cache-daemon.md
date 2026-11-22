# The gh credential-cache daemon (code#10742)

`fleet gh` resolves its rotation tokens (personal token, then the three
GitHub Apps' installation tokens) through a credential cache daemon when
one answers. Read this before debugging a slow `fleet gh`.

## Fallback order (fixed in the client)

1. Local daemon — `127.0.0.1:4802`
2. Laptop daemon — `100.99.113.103:4802` (tailnet; only reachable while
   that host's tailnet leg is up)
3. In-process resolution — always works, often slowly: every call
   re-walks the secrets chain, and when the secrets broker is cold or
   dead its retry budget burns ~120s per call. Measured on a Muse
   sandbox (2026-11-22): one `fleet gh issue view` took **2m02s**
   in-process vs **0.7–2.2s** through a warm local daemon.

`fleet gh cache-status` probes the configured daemons and prints hit/miss
counts. The "fleet gh: no cache daemon answered … resolving in-process"
stderr notice means every daemon probe failed — treat it as a bug to fix,
not background noise.

## The rule

**A local cache daemon runs at all times on any host that runs
`fleet gh`.** The verb is `fleet gh cache-serve` (default bind
127.0.0.1:4802; `--bind` / `FLEET_GH_CACHE_BIND` to change). The cache is
memory-only: a restart is a cold cache, never a stale one, so keeping it
alive is cheap and safe. On Muse sandboxes, `~/tools/init-ensure.sh`
keeps it running in the same ensure pattern as the session proxy and the
sandbox-local secrets broker (pgrep/port gate, `setsid nohup` spawn,
best-effort) — if `fleet gh` is slow on a sandbox, run `init-ensure.sh`
and check `fleet gh cache-status` before anything else. Note the secrets
broker must also be alive: the daemon mints through the same secrets
chain, and a dead broker makes even the daemon's first resolutions slow.

## The `personal` op returns the secret, not a token

Wire-contract asymmetry, load-bearing: the daemon's `jwt` and
`token` ops return finished tokens, but its `personal` op returns
the resolved SECRET ITSELF (the whole `github-cli.hosts-yml` file)
in the response's `token` field — the daemon is a dumb secret cache
for this op and never learns which secret "personal" means. The
client must parse the `oauth_token` out of whichever copy answered
(daemon or in-process). `FleetTokenProvider::token_for` does this
in one place (`personal_token()` in `gh.rs`); do not reintroduce a
path that uses a `personal` response as a finished token.

That exact mistake broke every `fleet gh` WRITE on 2026-11-22
while reads looked fine. The daemon path skipped the parse and
handed gh the 183-char hosts.yml as `GH_TOKEN`; gh refused it
client-side (`net/http: invalid header field value for
"Authorization"`). Reads masked it — a read fails over to an App
token on any failure — but writes repeat only on pre-execution
refusals, so the header error surfaced as-is. Shape evidence (never
values): the injected `GH_TOKEN` via the daemon was 183 chars with
newlines/colons (a YAML file); in-process it was the clean 40-char
`gho_` token. Diagnostic that pins it in one step: point
`FLEET_GH_BIN` at a stub that reports the injected token's length
and character classes, and compare daemon vs `--no-cache`. Fix:
PR #12500 (branch `xaltha/fleet-gh-personal-daemon-fix`) — parse at
one convergence point on both paths, regression test
`personal_token_is_the_parsed_token_not_the_secret`. Until #12500
merges and a fixed build is what `~/tools/fleet` runs, the installed
fleet still carries the defect: use `FLEET_GH_RAW=1 fleet gh` for
writes in the meantime.

Related wart, same symptom family: invoking `~/tools/fleet gh`
directly (not through the `gh` PATH shim) leaves `FLEET_GH_BIN`
unset, so the client's child `gh` resolves through PATH to the
shim, which re-enters `fleet gh` one layer deep — the nested layer
re-resolves through the daemon and drops outer flags such as
`--no-cache`. The shim's own entry pins `FLEET_GH_BIN` to the real
gh and does not nest. When a direct invocation behaves as if a
flag was ignored, check for the double `attempt 1` trace line.

## Fallback stickiness

This build's client has **no** stickiness: `gh_cache::fetch_any` tries
every address in order on every call (2s connect timeout per address) and
keeps no negative cache or cooldown — there is no config knob for it. So
stickiness is provided at the daemon-keeping layer, not the client layer:
an always-on local daemon means the preferred path is essentially never
dead, and the ensure prelude revives it within one run when it dies, so
calls stay on the fast working path instead of re-probing dead ones.
Do not "fix" slowness by pointing `FLEET_GH_CACHE` at the laptop only —
that makes every call depend on the flaky tailnet leg. If per-call
re-probing ever becomes the bottleneck on a host whose local daemon
cannot run, that is a client change (add a short negative-cache TTL in
`fetch_any`), not a config change.
