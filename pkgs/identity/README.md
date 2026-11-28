```
LINK_HOST=192.168.2.40
curl -L default.computer | bash -s -- "$LINK_HOST"
```

## Building

This module requires a build tag — the charm fork it depends on keeps its
sqlite/libsql migration variants behind tags, so a plain `go build ./...`
fails on undefined `Migration0003…`/`Migration0005…`/`Migration0006…` symbols:

```sh
go build -tags sqlite ./...   # local sqlite storage
go build -tags libsql ./...   # libsql storage
```

CI builds and vets both variants on every `pkgs/identity` change
(`identity-build.yml`, #340). The `build-sqlite.ps1` / `build-libsql.ps1`
helpers derive the same tag from their filename.

## Container images

The root flake exports `identity-image` and `identity-provider-image`. The
first builds the `sqlite`-tagged `charm` binary as a Nix package and places its
closure, CA bundle, data directory and service ports in a scratch-based OCI
image. The provider image contains the Bash runtime, `tcpserver`, and its
explicit command dependencies; its legacy `link` route still expects the
repository runtime layout described by the scripts.

Build both images with:

```sh
nix build .#packages.x86_64-linux.identity-image \
  .#packages.x86_64-linux.identity-provider-image
```

`web/` is not a separate executable or image target. The main `charm` server
embeds and starts that Go package. Its generated Gowebly Dockerfile is
documented as an exception in [`web/CONTAINER-EXCEPTION.md`](web/CONTAINER-EXCEPTION.md).
