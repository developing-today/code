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
