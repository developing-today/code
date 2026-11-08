#!/usr/bin/env bash
# openapi-sdk-check.sh — generate one SDK client from /v1/openapi.json and
# compile it (#1851, the named remainder after the docs pages in #2108/#2109).
#
# Why: mcpx publishes its HTTP surface as an OpenAPI document (GET
# /v1/openapi.json, generated in internal/api/openapi.go). The docs tell
# people to generate clients from it, but nothing in CI ever did — so a
# spec change that no generator can turn into compiling code would break
# every SDK consumer silently. This check closes that gap:
#
#   1. Ask the CLI for the current spec (`mcpx openapi`) — the same document
#      the server publishes, from the same code.
#   2. Generate a full client (models + client) with a PINNED generator
#      version (oapi-codegen, OPENAPI_SDK_GEN_VERSION) into a throwaway
#      module.
#   3. `go build` the generated package. Generation or compile failure
#      fails the check, which is exactly spec drift breaking an SDK.
#
# Two generator collisions in the current spec are worked around in the
# spec COPY this check feeds the generator; the published spec is never
# touched, and neither workaround changes the wire format:
#
#   - Path parameter named `server` (/v1/call/{server}/{tool},
#     /v1/writes/{server}/clear): oapi-codegen request builders take
#     `server string` as their first parameter, so a path parameter with
#     the same name redeclares it and the client does not compile. The
#     copy renames the template variable to `serverName` — a path
#     template variable name is a label, not part of the URL structure.
#   - Enum constant `Client` (SettingsListParamsScope = "client"):
#     collides with the generated `Client` struct. The generator config
#     below renames the generated client type to `APIClient`
#     (output-options.client-type-name).
#
# Deliberately no publishing and no committed generated code: the point
# is the proof, not an artifact. The generator runs via
# `go run <module>@<version>`, so the pin is in this file and the module
# cache makes CI runs reproducible.
#
# Usage: scripts/openapi-sdk-check.sh   (from anywhere; must have Go)
# Env:   OPENAPI_SDK_GEN_VERSION  override the pinned generator version
set -euo pipefail

PKG_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GEN_VERSION="${OPENAPI_SDK_GEN_VERSION:-v2.4.1}"
GENERATOR="github.com/oapi-codegen/oapi-codegen/v2/cmd/oapi-codegen@${GEN_VERSION}"

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/mcpx-sdk-check.XXXXXX")"
cleanup() { rm -rf "$WORK_DIR"; }
trap cleanup EXIT

echo "==> generating the current /v1 OpenAPI document"
(
  cd "$PKG_DIR"
  go run ./cmd/mcpx openapi
) >"$WORK_DIR/openapi.json"

python3 - "$WORK_DIR/openapi.json" "$WORK_DIR/openapi-sdk.json" <<'PY'
import json, sys
doc = json.load(open(sys.argv[1]))
assert doc.get("openapi"), "spec has no openapi version"
paths = doc.get("paths") or {}
assert paths, "spec has no paths"
# Rename the `server` path parameter in the generator copy (see header):
# oapi-codegen request builders already take a `server string` parameter.
renamed = {}
for path, item in paths.items():
    for op in (item or {}).values():
        if isinstance(op, dict):
            for param in op.get("parameters") or []:
                if param.get("name") == "server" and param.get("in") == "path":
                    param["name"] = "serverName"
    renamed[path.replace("{server}", "{serverName}")] = item
doc["paths"] = renamed
json.dump(doc, open(sys.argv[2], "w"))
print(f"==> spec ok: {len(paths)} paths, openapi {doc['openapi']}")
PY

cat >"$WORK_DIR/oapi-codegen.yaml" <<'EOF'
package: sdkcheck
generate:
  models: true
  client: true
output-options:
  client-type-name: APIClient
EOF

echo "==> generating an SDK client with oapi-codegen ${GEN_VERSION}"
(
  cd "$WORK_DIR"
  go run "$GENERATOR" \
    -config oapi-codegen.yaml \
    -o client.gen.go \
    openapi-sdk.json
)

echo "==> compiling the generated client"
(
  cd "$WORK_DIR"
  go mod init sdkcheck >/dev/null 2>&1
  go mod tidy >/dev/null 2>&1
  go build ./...
)

echo "==> OK: /v1/openapi.json generates a client that compiles"
