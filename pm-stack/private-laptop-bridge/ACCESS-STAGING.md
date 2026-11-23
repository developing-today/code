# No-WARP Cloudflare Access path

This is the selected source design for the fixed staging bridge. It replaces
the earlier Gateway/WARP and Tunnel sketches in `STAGING.md`; those sketches
are historical and must not be followed for this deployment. The two clients
make outbound HTTPS polling requests to one Cloudflare Access protected Worker
custom hostname. There is no listener on the AMD laptop and no generic proxy.

```text
Aster caller runner -- outbound HTTPS + Access service token + Ed25519 -->
                    fixed Access hostname --> Worker default fetch -->
                    SQLite Durable Object
AMD non-root client -- outbound HTTPS + Access service token + Ed25519 -->
```

Proposed fixed host: `laptop-bridge-staging.security.cab`. This differs from
the rejected `private-bridge-staging.security.cab`. Root must independently
verify exact hostname ownership, DNS/wildcard behavior, and the Access app
target before any provider configuration. No wildcard or unrelated DNS record
is to be changed. This draft does not prove that the hostname is available or
that an Access application already exists.

## Two independent authentication checks

Each request must pass both layers:

1. Cloudflare Access accepts one of two separate Service Auth credentials: one
   held by the approved Aster runner, one by the AMD client. No browser or
   identity-provider policy is included. Keep strict service-token
   authentication enabled. Do not share or log either Client Secret.
2. The Worker verifies `Cf-Access-Jwt-Assertion` itself. It pins the exact
   `https://<team>.cloudflareaccess.com` issuer, exact app audience, `RS256`,
   `kid`, fetched JWKS, `type=app`, `sub=""`, `common_name`, `iat`, `exp`, and
   optional `nbf`. The Worker also requires the runtime-verified
   `ctx.access.aud` to match the configured audience. The signed `common_name`
   selects exactly one unrevoked principal; the request's
   `x-bridge-principal` must equal that principal's ID before forwarding. The
   Worker does not rely on the original `CF-Access-Client-Id` header, which is
   not guaranteed to reach the Worker. A token for one service cannot assume
   the other principal's role.
   JWKS are fetched only from the exact configured issuer, redirects are
   rejected, cache lifetime is five minutes, and an unknown key or failed
   refresh after expiry fails closed. `exp` must be strictly in the future;
   `iat` cannot be more than 30 seconds in the future, and optional `nbf` uses
   the same bounded future skew. No undocumented maximum `exp - iat` is
   assumed; Access policy token lifetime must be reviewed independently.
3. The existing bridge check remains independent: Ed25519 signs the request
   method, exact path, body, audience, principal, timestamp, and nonce. The
   Durable Object checks role, device, operation scope, quota, replay, lease,
   source freshness and revocation. Access authentication does not grant a
   bridge operation.

Before forwarding to the Durable Object, the Worker removes all `cf-access-*`
headers, `Cookie`, and `Authorization`. It never uses caller-supplied identity
headers. Denial logs contain only fixed status and reason codes.

The default Worker fetch rejects a missing or mismatched exact HTTPS hostname,
port, route or query and Upgrade requests before JWKS or Durable Object calls;
rejects missing runtime Access context, mismatched context audience, malformed,
forged, wrong-issuer, wrong-audience, expired, cross-client, stale-key, or
wrong-algorithm tokens;
and fails closed when the auth configuration or JWKS are unavailable. Redirects
are not followed by either client. The deployment must keep `workers_dev` and
preview URLs disabled and bind only the approved custom hostname. Do not create
additional custom domains, routes, Worker aliases, public management endpoints,
or a direct Worker origin. Cloudflare Access must be configured for the exact
hostname before it is attached to the Worker. The Worker-side JWT validation
is also required because the edge Access rule alone does not prove that no
alternate path can reach the code.

The custom hostname will resolve through Cloudflare; “private” here means
service-authenticated and operation-limited, not a DNS-hidden service. No
WARP, Cloudflare Tunnel, Gateway rule, inbound laptop port, or host network
change is part of this design. A Tunnel would expose a local origin and add a
different route, so it is not used for this Worker control plane.

## Resource and identity checklist

- Worker: `private-laptop-bridge-staging`; fixed hostname above, Access app
  restricted to service-token authentication, `workers_dev=false`, previews
  false, no schedules or other routes/domains.
- Access issuer and audience: provisioned for that exact app, stored as
  non-secret Worker configuration from protected environment variables
  `CF_BRIDGE_ACCESS_ISSUER` and `CF_BRIDGE_ACCESS_AUDIENCE`. They are required
  by the build/readback contract and are not yet verified or set here.
- Access identities: two separately named Service Auth credentials with
  independent expiration and revocation; Aster caller and AMD device. Keep
  their IDs in the role registry and each Client Secret only in that client's
  owner-only config. No secret is stored in Cloudflare Worker variables.
- Bridge principals: distinct Ed25519 public keys and unique Access Client IDs;
  caller scopes only the four read operations; device scopes only approved
  operations for device `amd`. Revoke by disabling the Access token and
  revoking its Worker principal. Never extend either identity beyond the
  approved one-hour pilot without a new review.
- Runtime: outbound HTTPS only from both processes. Aster's Node runtime and
  AMD's non-root client use the fixed origin and no redirects. Their local
  Access service credentials and Ed25519 private halves remain on their own
  hosts. Never share a signing key, Access Secret, token-bearing logs, or
  `.env` artifact.

## Verification gates

Before attaching the hostname or enabling the Worker, an owner must review the
exact account and Access app, confirm the service-token policy includes only
the two approved credentials, verify no conflicting wildcard/custom-domain
mapping, and read back the Worker name and immutable ID. The release environment
must pin that ID; a same-name Worker with another ID fails. Do not infer
protection from DNS, a browser login, or a successful synthetic request.

The private owner proof consumed by the release gate must record the Access app
ID, exact hostname/issuer/audience, service-token-only mode, disabled browser
login, zero bypass/non-service allow rules, and exactly two distinct approved
service-token client IDs. It contains IDs, never Client Secrets. The owner must
independently derive that normalized record from the Access app and policy
readback; a manually asserted proof without that readback is not valid.

Offline tests cover valid RS256/JWKS verification, forged signatures, wrong
issuer/audience/client, cross-role ID, wrong host, expiry, stale/unknown keys,
failed JWKS refresh, Access header stripping, principal revocation, replay,
scopes, and denied operations. Add a credential-free hosted smoke against the
exact Access app only after its host and audience are reviewed. Then prove a
separate real Node process on each approved runner can call health and obtain a
fresh queue snapshot. A user/session-proxy queue timeout or synthetic empty
snapshot is not a valid idle result.

## Current stop conditions

The source has no verified Access issuer, audience, service-token Client IDs,
Access policies, exact hostname ownership, or deployed Worker. The protected
Access variables have no verified values and the bridge is disabled. No
credential, DNS, Worker, Access app, or client state was changed. The first
Worker must be created locally by an account owner as a metadata-only disabled
object; Actions must use only its pre-pinned Individual Workers Editor scope.
The pinned `cf deploy --provision` behavior for an empty SQLite Durable Object
binding, exact Worker Editor permissions, and preservation of disabled ingress
has not been proven against Cloudflare. Do not call it compatible until that
provider contract is independently verified. See the dormant bootstrap source
and the staging approval packet for the separate release and rollback gates.

Cloudflare documents service token JWT `common_name` as the Client ID and
requires signature, issuer, audience, and time validation at an origin:
[Application token](https://developers.cloudflare.com/cloudflare-one/access-controls/applications/http-apps/authorization-cookie/application-token/),
[Validate JWTs](https://developers.cloudflare.com/cloudflare-one/access-controls/applications/http-apps/authorization-cookie/validating-json/),
[Service tokens](https://developers.cloudflare.com/cloudflare-one/access-controls/service-credentials/service-tokens/).
