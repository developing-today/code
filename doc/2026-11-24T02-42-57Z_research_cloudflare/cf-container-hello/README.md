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
  container-served apart

Findings (full story in pkgs/lab-ops-hub README, "Containers: FIXED"):
the platform and the `lite` type were fine; the Nix v3 image shape
(relative Cmd, no PATH, no shell) and the Oct-20-created container
application were both broken. `image/` holds the fixed-shape (v4)
crawler image recipe used for the probe.
