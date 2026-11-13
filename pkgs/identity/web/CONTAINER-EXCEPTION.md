# Container status

The checked-in Gowebly-generated `Dockerfile` is retained as an explicit
exception, not as a deployable image target. It has no standalone Go entry
point: `web` is a library package embedded and started by the main `charm`
server. Its generated compose context also lacks the module's `go.mod` and
`go.sum`, so the generated build cannot succeed as written. The repository has
no tracked Gowebly generator configuration or command to repoint.

**Generator:** Gowebly (`https://gowebly.org`).
**Owner:** repository maintainers.
**Reason:** this is stale generated scaffolding for a non-standalone package;
the live web server is included in the Nix-built `identity` image instead.
