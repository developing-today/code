#!/usr/bin/env bash
# tier2-containers.sh — the tier-2 container legs for #1987.
#
# Builds the static (CGO_ENABLED=0, unwrapped) mcpx binary once, then runs
# it in the minimal/extended images the tier-2 set names, asserting the
# surface each image can actually support:
#
#   alpine-nonexec             alpine, no JS runtime: --version works and
#                              exec refuses cleanly ("no usable JavaScript
#                              runtime") instead of crashing
#   alpine-bunmusl-exec        alpine + bun's musl build: exec runs a script
#   scratch-nonexec            FROM scratch (+ CA certs): --version + refusal
#   distroless-static-nonexec  gcr.io/distroless/static: --version + refusal
#   distroless-cc-deno-exec    gcr.io/distroless/cc + deno: exec runs a script
#
# Deliberately NOT a leg: alpine + nodejs exec. Verified 2026-10-30: mcpx's
# node path dies with ERR_NO_TYPESCRIPT under Alpine's nodejs build, so the
# leg would enshrine a failure; it is deferred with the finding recorded on
# #1987 until the node invocation works on musl node.
#
# Usage: scripts/tier2-containers.sh [leg ...]   (default: all legs)
# Needs: go, docker. Run by .github/workflows/mcpx-tier2-containers.yml.
set -euo pipefail

cd "$(dirname "$0")/.."

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "== building static mcpx =="
CGO_ENABLED=0 go build -trimpath -ldflags "-s -w -X main.version=0.1.0" \
  -o "$WORK/mcpx" ./cmd/mcpx
go version -m "$WORK/mcpx" | grep -q 'CGO_ENABLED=0'
file "$WORK/mcpx" | grep -q 'statically linked'
cp "$WORK/mcpx" "$WORK/ctx/mcpx" 2>/dev/null || { mkdir -p "$WORK/ctx"; cp "$WORK/mcpx" "$WORK/ctx/mcpx"; }
cd "$WORK/ctx"

STATE_ENV=(-e MCPX_STATE_DIR=/tmp/state -e MCPX_CACHE_DIR=/tmp/cache)

expect_refusal() {
  # $1 = image, rest = entrypoint args prefix (may be empty)
  local image="$1"; shift
  local out
  out="$(timeout 120 docker run --rm "${STATE_ENV[@]}" "$image" "$@" exec 'console.log(1)' 2>&1 || true)"
  echo "$out" | head -2
  grep -q 'no usable JavaScript runtime' <<<"$out"
}

leg_alpine_nonexec() {
  echo "== leg: alpine-nonexec =="
  timeout 60 docker run --rm -v "$PWD/mcpx:/mcpx:ro" "${STATE_ENV[@]}" \
    alpine:3.21 /mcpx --version
  local out
  out="$(timeout 120 docker run --rm -v "$PWD/mcpx:/mcpx:ro" "${STATE_ENV[@]}" \
    alpine:3.21 /mcpx exec 'console.log(1)' 2>&1 || true)"
  echo "$out" | head -2
  grep -q 'no usable JavaScript runtime' <<<"$out"
}

leg_alpine_bunmusl_exec() {
  echo "== leg: alpine-bunmusl-exec =="
  cat > Containerfile.bunmusl <<'EOF'
FROM oven/bun:1-alpine AS bun
FROM alpine:3.21
RUN apk add --no-cache ca-certificates libstdc++ libgcc
COPY --from=bun /usr/local/bin/bun /usr/local/bin/bun
COPY mcpx /usr/local/bin/mcpx
ENV MCPX_STATE_DIR=/tmp/state MCPX_CACHE_DIR=/tmp/cache MCPX_SCRIPT_RUNTIME=bun
ENTRYPOINT ["mcpx"]
EOF
  docker build -q -f Containerfile.bunmusl -t mcpx-tier2-bunmusl . >/dev/null
  timeout 120 docker run --rm mcpx-tier2-bunmusl \
    exec 'console.log("tier2-bunmusl-ok")' | grep -q 'tier2-bunmusl-ok'
}

leg_scratch_nonexec() {
  echo "== leg: scratch-nonexec =="
  cat > Containerfile.scratch <<'EOF'
FROM alpine:3.21 AS certs
FROM scratch
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY mcpx /mcpx
ENV MCPX_STATE_DIR=/tmp/state MCPX_CACHE_DIR=/tmp/cache
ENTRYPOINT ["/mcpx"]
EOF
  docker build -q -f Containerfile.scratch -t mcpx-tier2-scratch . >/dev/null
  timeout 60 docker run --rm mcpx-tier2-scratch --version
  expect_refusal mcpx-tier2-scratch
}

leg_distroless_static_nonexec() {
  echo "== leg: distroless-static-nonexec =="
  cat > Containerfile.dstatic <<'EOF'
FROM gcr.io/distroless/static-debian12
COPY mcpx /mcpx
ENV MCPX_STATE_DIR=/tmp/state MCPX_CACHE_DIR=/tmp/cache
ENTRYPOINT ["/mcpx"]
EOF
  docker build -q -f Containerfile.dstatic -t mcpx-tier2-dstatic . >/dev/null
  timeout 60 docker run --rm mcpx-tier2-dstatic --version
  expect_refusal mcpx-tier2-dstatic
}

leg_distroless_cc_deno_exec() {
  echo "== leg: distroless-cc-deno-exec =="
  cat > Containerfile.dcc <<'EOF'
FROM denoland/deno:debian AS deno
FROM gcr.io/distroless/cc-debian12
COPY --from=deno /usr/bin/deno /usr/local/bin/deno
COPY mcpx /usr/local/bin/mcpx
ENV MCPX_STATE_DIR=/tmp/state MCPX_CACHE_DIR=/tmp/cache MCPX_SCRIPT_RUNTIME=deno HOME=/tmp
ENTRYPOINT ["/usr/local/bin/mcpx"]
EOF
  docker build -q -f Containerfile.dcc -t mcpx-tier2-dcc . >/dev/null
  timeout 120 docker run --rm mcpx-tier2-dcc \
    exec 'console.log("tier2-deno-ok")' | grep -q 'tier2-deno-ok'
}

legs=("$@")
if [ "${#legs[@]}" -eq 0 ]; then
  legs=(alpine-nonexec alpine-bunmusl-exec scratch-nonexec
        distroless-static-nonexec distroless-cc-deno-exec)
fi
for leg in "${legs[@]}"; do
  case "$leg" in
    alpine-nonexec) leg_alpine_nonexec ;;
    alpine-bunmusl-exec) leg_alpine_bunmusl_exec ;;
    scratch-nonexec) leg_scratch_nonexec ;;
    distroless-static-nonexec) leg_distroless_static_nonexec ;;
    distroless-cc-deno-exec) leg_distroless_cc_deno_exec ;;
    *) echo "unknown leg: $leg" >&2; exit 2 ;;
  esac
done
echo "tier2-containers: all requested legs passed"
