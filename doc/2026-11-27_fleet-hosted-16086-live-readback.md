# fleet-hosted deployment and RP acceptance readback (#16086)

Lane `oc-code-01`, task 26197, 2026-11-27. Read-only production checks; no deployment, route, secret, binding, or business-data mutation.

## Deployment exists

The original local-only #12583/#12822 receipt is superseded by Cloudflare-first #14866/#14870. [Actions release 45781053761](https://github.com/developing-today/code/actions/runs/45781053761) deployed commit `c3add6907645e212685ea4a4b4317652d7c74579`.

Authenticated Cloudflare API readback on 2026-11-27, using `fleet secrets get cloudflare.api-token-day-to-day` in memory:

- Account: `234b866b99ce2b142d7963d8d139cbfa`, Developing.today.llc.domains.0@gmail.com.
- API: `GET /accounts/234b866b99ce2b142d7963d8d139cbfa/workers/scripts/fleet-hosted/deployments`.
- Current deployment: `e98e3a47-dfd4-4c57-a300-500cea011168`, created `2026-11-26T08:13:54.993516Z`.
- Serving version: `0fd14383-94ec-4e4b-b616-ab32166096e8`, **100%**.
- Deployment message: `GHA deploy c3add6907645e212685ea4a4b4317652d7c74579 (#14941)`.
- Previous version in deployment history: `ac15ba6d-97a4-43b1-9905-a5e7d6c4b312`.

Rollback uses `.github/workflows/cf-worker-versions-release.yml` from main, `worker=fleet-hosted`, `operation=rollback`, `version_id=<previous version UUID>`. Historical rehearsal/restore evidence is [#14870 comment 7155701971](https://github.com/developing-today/code/issues/14870#issuecomment-7155701971). No rollback was exercised in this run. Package version `0.1.0` alone is not deployment identity; use the authenticated version record above.

## Live HTTP checks

At approximately 10:54Z on 2026-11-27, ordinary curl requests (no custom User-Agent) to `https://fleet-hosted.developing-today.workers.dev` returned:

| Request | Result | Oracle |
| --- | --- | --- |
| `GET /health` | 200; `status=ok`, `service=fleet-hosted`, `version=0.1.0`; `auth.rp_configured=false`, `auth.tenancy_configured=false` | Hosted service identity and explicit configuration flags |
| `GET /capabilities` | 200; `build.name=fleet-hosted`, `profile=hosted`, `edition=hosted`, `version=0.1.0` | Hosted build identity, not native Fleet identity |
| `POST /v1/queue/health`, `{}`, absent bearer | 503 `auth_not_configured` | Fail-closed configuration; **not** missing-token validation proof |
| Same read-only request, `Bearer invalid.invalid.invalid` | 503 `auth_not_configured` | Fail-closed configuration; **not** invalid-token validation proof |

Ordinary Python urllib requests at 10:53Z instead returned Cloudflare 403/error 1010 on all four requests. This client-dependent edge rejection is recorded, not treated as application auth evidence or fixed by spoofing a User-Agent. curl success does not establish ordinary-client compatibility for every client.

## What remains

Deployment/public identity and historical rollback receipts exist. **Valid and invalid production RP-token acceptance remains unproven.** Repeating the deployment would not configure it.

Current `pkgs/fleet/hosted-worker/src/config.ts` supplies tenancy through `TENANCY_FIXTURE_JSON`, documented as local/test plumbing. Production requires a real tenancy directory, Worker-reachable issuer/JWKS, audience/subject binding, and a live production token. Do not enable test-fixture identities on the shared public Worker merely to satisfy this check: authentication also gates implemented business doors.

The provider is not wholly unimplemented: [#13409](https://github.com/developing-today/code/issues/13409) closed with live loopback issuer `http://127.0.0.1:8911` and PKCE/JWKS proof. That receipt is not proof of a public Worker-reachable issuer or fleet-hosted tenancy integration. [#9077](https://github.com/developing-today/code/issues/9077) and [#13416](https://github.com/developing-today/code/issues/13416) still track the SSO programme/consumer proof. #16086 remains open for its specific live RP acceptance, independent of those programmes' overall completion.

A read-only verifier accompanies this receipt. Its incomplete result must remain nonzero when auth is unconfigured, a valid token is absent, or an edge response prevents application verification. It must never turn the 503 above into a successful invalid-token check.
