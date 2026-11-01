#!/usr/bin/env bash
# darwin-compile-check.sh — cross-compile gate for darwin/arm64 (#1988).
#
# Nothing else in CI ever compiles mcpx's test packages for darwin, so a
# Linux-only constant in a _test.go file is invisible until someone runs
# the suite on a Mac. That is exactly #237's break. This check costs
# ~30s on the existing ubuntu job and needs no macOS runner: it builds
# the module for darwin/arm64 and compiles every package's test binary.
#
# It does NOT run the tests (that needs darwin hardware; #1988/#237) and
# it does NOT fix any darwin portability defect (#237 owns those).
set -euo pipefail
cd "$(dirname "$0")/.."

# The single exception list. A package named here may fail the darwin
# test compile without failing this gate; every other package must
# compile. Empty since #237 landed (the internal/daemon entry — its
# takeover_test.go used the Linux-only syscall.SOCK_CLOEXEC — was the
# last one). Add an entry only with a tracking issue.
DARWIN_COMPILE_EXCEPTIONS="${DARWIN_COMPILE_EXCEPTIONS-}"

export GOOS=darwin GOARCH=arm64 CGO_ENABLED=0

echo "darwin/arm64: go build ./..."
go build ./...

failed=()
skipped=()
start=$SECONDS
while IFS= read -r pkg; do
  [ -n "$pkg" ] || continue
  if go test -c -o /dev/null "$pkg" 2>"/tmp/darwin-compile-err.txt"; then
    case " $DARWIN_COMPILE_EXCEPTIONS " in
      *" $pkg "*) echo "note: $pkg now compiles for darwin — remove it from DARWIN_COMPILE_EXCEPTIONS" ;;
    esac
  else
    case " $DARWIN_COMPILE_EXCEPTIONS " in
      *" $pkg "*)
        echo "known exception (see list in this script): $pkg"
        sed 's/^/    /' "/tmp/darwin-compile-err.txt" | head -3
        skipped+=("$pkg")
        ;;
      *)
        echo "FAIL darwin test compile: $pkg"
        sed 's/^/    /' "/tmp/darwin-compile-err.txt"
        failed+=("$pkg")
        ;;
    esac
  fi
done < <(go list -f '{{if or .TestGoFiles .XTestGoFiles}}{{.ImportPath}}{{end}}' ./...)

echo "darwin test-compile scan took $((SECONDS - start))s; exceptions skipped: ${#skipped[@]}"
if [ "${#failed[@]}" -gt 0 ]; then
  echo "darwin compile gate FAILED for ${#failed[@]} package(s):"
  printf '  %s\n' "${failed[@]}"
  echo "Fix the darwin compile, or — only with a tracking issue — add the package to DARWIN_COMPILE_EXCEPTIONS in scripts/darwin-compile-check.sh."
  exit 1
fi
echo "darwin compile gate passed"
