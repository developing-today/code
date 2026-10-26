# cf-container-hello — known-good container reference

Clean-room test Worker built 2026-10-25 to debug the Lab Ops Hub
container start failures. It runs Cloudflare's OFFICIAL hello-world
container image (github.com/cloudflare/hello-world-container-image;
Docker Hub `cloudflare/hello-world`, linux/amd64 manifest copied
byte-for-byte into our account registry as `hello-world:v1`).

Routes:

- `/` — singleton HelloContainer (official image, default instance type)
- `/lite` — HelloLite (official image, instance_type "lite")
- `/probe` — CrawlerProbe: the Lab Ops Hub crawler image inside a fresh
  container application (the isolation probe that split "our image" from
  "their platform" from "the wedged Oct-20 application")
- `/worker-marker` — plain Worker response, to tell Worker-served from
  container-served apart (the only public route; it touches no container)

## Auth — required before any deploy (#955)

Every container-forwarding route (`/probe`, `/lite`, `/name/…`, `/`)
requires `Authorization: Bearer <API_TOKEN>` (or `x-api-token`), where
`API_TOKEN` is a Worker secret:

```
npx wrangler secret put API_TOKEN
```

The gate fails closed, mirroring lab-ops-hub (#441): if the secret is
unset the Worker returns 503 for every container route, and an
unauthenticated request returns 401 before any container starts. Do not
deploy without setting the secret — and do not remove the gate: before
#955 this Worker forwarded `/probe` to the crawler with no auth at all.

## Crawler image — no local copy (#955)

There is deliberately no `image/` directory in this package any more.
The old `image/app.py` was a stale, pre-hardening copy of the crawler
(it fetched any caller-supplied URL, `file://` included, with no SSRF
policy), and `image/container-image-v4.nix` duplicated the production
recipe except for that stale copy. CrawlerProbe now uses the production
image directly: `lab-ops-hub-crawlercontainer:v5`, built by
`pkgs/lab-ops-hub/container-image.nix` from the hardened
`pkgs/lab-ops-hub/container/app.py` (`validate_crawl_url`: public
http(s) only, no localhost/private/non-global targets, redirect targets
re-validated). Rebuild/repush that image from pkgs/lab-ops-hub if the
crawler changes; never fork a copy into this package again.

Findings (full story in pkgs/lab-ops-hub README, "Containers: FIXED"):
the platform and the `lite` type were fine; the Nix v3 image shape
(relative Cmd, no PATH, no shell) and the Oct-20-created container
application were both broken. The fixed-shape (v4, now v5 in
pkgs/lab-ops-hub) crawler image recipe is what the probe runs.

## Tests

```
npm test   # python3 -m unittest discover -s test -p "test_*.py"
```

`test/test_probe_hardening.py` pins the #955 regression: the production
validator the probe now runs refuses `file://`, loopback, private-IP
and redirect-to-private targets; no stale crawler copy exists in this
package; wrangler points CrawlerProbe at the hardened `:v5` image; and
the Worker's fail-closed gate sits before every container forward.
