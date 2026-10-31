#!/usr/bin/env bash
# packaging-test.sh -- test the mcpx artifact users install, not the
# source tree (#1981). Everything below runs the wrapped binary from a
# built package output with an empty HOME:
#
#   nix build .#packages.x86_64-linux.mcpx
#   pkgs/mcpx/packaging-test.sh ./result
#
# The same script is the flake check checks.mcpx-packaging (wired in
# lib/default.nix, run by the mcpx-nixbuild workflow), so the hand-run
# command and CI can never drift apart. Source-tree suites
# (internal/e2e and friends) complement this; they never see $out.
set -euo pipefail

out="${1:?usage: packaging-test.sh <mcpx package output path>}"
bin="$out/bin/mcpx"
fail() {
  echo "packaging-test: FAIL: $*" >&2
  exit 1
}
note() { echo "packaging-test: $*"; }

[ -x "$bin" ] || fail "$bin is missing or not executable"
[ -f "$out/bin/.mcpx-wrapped" ] || fail "the wrapped binary is missing: $out/bin/mcpx is not the packaged wrapper"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export HOME="$work/home"
mkdir -p "$HOME"
# State belongs under the empty HOME, never in a developer's real one.
# XDG_STATE_HOME / XDG_CACHE_HOME override the HOME-based defaults in
# the daemon's path resolution, so they go too: an inherited XDG dir
# would leak state out of the empty HOME (Codex, PR #2994).
unset MCPX_STATE_DIR MCPX_CACHE_DIR XDG_STATE_HOME XDG_CACHE_HOME

# --version, from the store path, with no pre-existing state.
version_out="$("$bin" --version)" || fail "mcpx --version did not run"
version="$(printf '%s' "$version_out" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+[^ ]*' | head -1)"
[ -n "$version" ] || fail "mcpx --version printed no version: $version_out"
note "version: $version_out"

# The wrapper's PATH, verified from inside the process: run doctor
# with a PATH that has none of the runtimes on it. Whatever doctor
# finds (it uses exec.LookPath in-process) arrived via the wrapper's
# --suffix PATH from package.nix's postInstall. Read the results
# through doctor's JSON interface, never its human output
# (pkgs/mcpx/AGENTS.md: "Do not parse human output; use
# `mcpx --json <command>`" — Codex, PR #2994).
doctor_json="$(env -i HOME="$HOME" PATH=/nonexistent "$bin" --json doctor 2>/dev/null || true)"
[ -n "$doctor_json" ] || fail "mcpx --json doctor produced no output"
runtime_check="$(printf '%s' "$doctor_json" | grep -o '"name":"runtime"[^}]*}' || true)"
[ -n "$runtime_check" ] || fail "mcpx --json doctor has no runtime check: $doctor_json"
for rt in deno bun node; do
  printf '%s' "$runtime_check" | grep -q "$rt (/nix/store/" ||
    fail "doctor (scrubbed PATH) did not find $rt on the wrapper PATH"
done
git_check="$(printf '%s' "$doctor_json" | grep -o '"name":"git"[^}]*}' || true)"
printf '%s' "$git_check" | grep -q 'git at /nix/store/' ||
  fail "doctor (scrubbed PATH) did not find git on the wrapper PATH"
note "wrapper PATH reaches deno, bun, node and git (doctor --json, scrubbed PATH)"

# Man page and completions, present in the built output, and the man
# page's .TH line carries the built version.
man="$out/share/man/man1/mcpx.1.gz"
[ -f "$man" ] || fail "man page missing: $man"
zcat "$man" >"$work/mcpx.1" || fail "man page does not decompress"
grep -m1 '^\.TH' "$work/mcpx.1" | grep -q "mcpx $version" ||
  fail "man page .TH line does not carry the built version $version"
for f in "share/bash-completion/completions/mcpx.bash" "share/zsh/site-functions/_mcpx" "share/fish/vendor_completions.d/mcpx.fish"; do
  [ -s "$out/$f" ] || fail "completion file missing or empty: $f"
done
note "man page (version $version) and bash/zsh/fish completions present"

# mcpx serve over stdio: initialize and server/discover answer, and
# tools/list returns the catalogue. Newline-delimited JSON-RPC in,
# the same out; the process exits when stdin closes.
req="$work/requests.jsonl"
resp="$work/responses.jsonl"
{
  printf '%s
' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"packaging-test","version":"0"}}}'
  printf '%s
' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  printf '%s
' '{"jsonrpc":"2.0","id":2,"method":"server/discover","params":{}}'
  printf '%s
' '{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}'
} >"$req"
timeout 60 "$bin" serve <"$req" >"$resp" 2>"$work/serve.err" ||
  fail "mcpx serve exited non-zero: $(cat "$work/serve.err")"
grep -q '"id":1' "$resp" || fail "initialize went unanswered"
grep '"id":1' "$resp" | grep -q '"serverInfo"' || fail "initialize answered without serverInfo"
grep '"id":2' "$resp" | grep -q '2026-07-28' ||
  fail "server/discover did not answer on the 2026-07-28 revision"
grep '"id":3' "$resp" | grep -q '"name":"mcpx_' ||
  fail "tools/list returned an empty catalogue"
note "stdio serve: initialize, server/discover (2026-07-28) and tools/list all answer"

note "PASS: the built artifact installs, starts cold, and serves"
