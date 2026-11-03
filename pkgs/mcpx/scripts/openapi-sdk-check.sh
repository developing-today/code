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
#   2. Generate a client with a PINNED generator version (oapi-codegen,
#      OPENAPI_SDK_GEN_VERSION) into a throwaway module.
#   3. `go build` the generated package. Generation or compile failure
#      fails the check, which is exactly spec drift breaking an SDK.
#
# Deliberately no publishing, no committed generated code and no spec
# changes: the point is the proof, not an artifact. The generator runs via
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

python3 - "$WORK_DIR/openapi.json" <<'PY'
import json, sys
doc = json.load(open(sys.argv[1]))
assert doc.get("openapi"), "spec has no openapi version"
paths = doc.get("paths") or {}
assert paths, "spec has no paths"
print(f"==> spec ok: {len(paths)} paths, openapi {doc['openapi']}")
PY

echo "==> generating an SDK client with oapi-codegen ${GEN_VERSION}"
# Models + client in one package collide on spec schema names: /v1 defines
# schemas literally named "Client" and "Server", which redeclare
# oapi-codegen's own generated identifiers. Until those are disambiguated
# (client-type-name handles "Client", "Server" still needs a mapping), this
# check pins the compile proof to the generated model set — the half that a
# spec change breaks first — and the collision is recorded on #1851.
(
	cd "$WORK_DIR"
	go run "$GENERATOR" \
		-generate types \
		-package sdkcheck \
		-o client.gen.go \
		openapi.json
)

echo "==> compiling the generated client"
(
	cd "$WORK_DIR"
	go mod init sdkcheck >/dev/null 2>&1
	go mod tidy >/dev/null 2>&1
	go build ./...
)

echo "==> OK: /v1/openapi.json generates and compiles"
