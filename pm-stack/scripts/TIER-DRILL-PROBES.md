# CF-12 tier probe observations

`python3 -I pm-stack/scripts/tier-drill-probe.py --base-url https://pm.security.cab --path /console/health --path /board > baseline.json`

This read-only collector is the first measurable slice of [CF-12 #14952](https://github.com/developing-today/code/issues/14952).
It only performs GETs. It never stops services, flips routes, changes write authority,
claims/completes queue tasks, or declares a drill successful. Explicit paths are required:
there is no claim that a built-in list covers every programme surface.

## Contract

- Required: `--base-url` is an HTTP(S) origin without credentials, and one or more
  `--path` origin-relative paths (queries allowed). Optional `--timeout` is a finite
  positive per-socket timeout in seconds, at most 120, default 10.
- All arguments are checked before requests. Unknown flags, abbreviations, stray
  positionals, path escapes and missing values exit 2 without emitting observations.
- Redirects are **not followed**: a Cloudflare Access login redirect must not become
  a green surface observation. Use an already authorised probe origin/session via an
  appropriate external mechanism; this script does not bypass Access.
- Stdout is one JSON record, including start/end times, URL, HTTP status, redirect
  location, content type, elapsed time and SHA-256 of up to 64 KiB of response body.
  Bodies are not persisted. `bodyTruncated` explicitly marks a sampled digest.
- Non-2xx or transport failures exit 1, but retain observations for all requested paths.
  All 2xx exits 0. **Neither exit 0 nor `httpSuccess` proves functional correctness**:
  a login/error page may itself return 200. Inspect surface-specific content separately.
- Every record is labelled `tier-probe-observations`, `provesDrill: false`. These
  are observations, not trusted attestations of an outage, restoration or serving tier.

## Using observations during a real drill

First verify the owning prerequisites and use a scoped, separately reviewed fault and
restoration procedure. Never stop a shared laptop or disable Cloudflare just to run this
collector. Run the same explicit path set from an **off-laptop seat** before the fault,
during the independently verified outage, and after verified restoration; save each JSON
record, the source commit, seat, exact command, outage/restoration timestamps and failures.

For laptop-off, cover all hosted surfaces and inspect laptop-sourced views for visible
last-known age or honest unavailability. For Cloudflare-off, prove the Cloudflare paths
are actually disabled and probe the tailnet origin (not a hosted-to-tunnel route flip).
A green baseline is neither drill. Queue claims/completions and D1 authority need CF-2's
separate controlled queue drill; this GET-only collector cannot prove them.

## Evaluating an evidence package

`python3 -I pm-stack/scripts/tier-drill-evaluate.py --before before.json --during during.json --after after.json --outage-start TS --outage-end TS --restored-at TS --seat SEAT --command CMD`

This read-only evaluator checks package completeness only: the same explicit
surface set in all three records, every record labelled `provesDrill: false`
(a `true` is rejected as a false completion claim), every observation carrying
an ordered `startedAt`/`finishedAt` pair, and an honestly ordered
outage window with the during record inside it and the after record after
restoration. Exit 0 with `verdict: complete`, exit 1 with named `gaps`,
exit 2 for invalid arguments or unreadable evidence. A complete verdict is
not drill success — it says the package is evaluable, so a reviewer can
judge the drill instead of hunting missing surfaces.

## Acceptance state at this slice

Design: `doc/2026-11-25_design_cloudflare-first-migration.md` §§6,8;
current issue instructions and drill plan on #14952.

| CF-12 criterion                   | State      | Evidence / remaining work                                                                                                             |
| --------------------------------- | ---------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| Three actual tier drills recorded | Unmeasured | Existing comments explicitly say NOT RUN; this collector adds reproducible observations only. CF-4–CF-7 remain open as of 2026-11-27. |
| M9 checklist complete             | Unchecked  | Serving/book verification and pipeline-written deployed commits still require live verification; no close-out claimed.                |
| Parent close-out summary          | Unchecked  | #14866 needs the eventual evidence-backed programme verdict, not a synthetic pass.                                                    |

No live drill, deployment or serving fact is changed by this slice.

## Test oracle

`python3 -I pm-stack/scripts/tier-drill-probe.test.py`

A controlled local HTTP server supplies 200, 302-to-login and 503 responses. Assertions
pin no redirect following, complete observations despite failures, explicit non-drill
labelling, transport errors, and rejection of invalid arguments before output/network.
The test uses no Cloudflare credentials and is run by `pm-stack-tests.yml`.

`python3 -I pm-stack/scripts/tier-drill-evaluate.test.py`

Synthetic probe records only — no network, no live services. Assertions pin acceptance
of a complete package, rejection of a dropped surface, of a tampered `provesDrill: true`
completion claim, of a during record outside the outage window, of an after record
predating restoration, of a foreign record kind, and of invalid arguments or unreadable
evidence (exit 2, no verdict on stdout) before any gap judgement. Also run by
`pm-stack-tests.yml`.
