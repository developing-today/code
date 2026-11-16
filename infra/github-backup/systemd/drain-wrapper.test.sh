#!/usr/bin/env bash
# Test for infra/github-backup/systemd/drain-wrapper.sh (#5222).
#
# Oracle: the wrapper's Bitwarden credential fallback chain, and
# nothing else. The wrapper must reach its drain step exactly when
# one of the credential branches yields a working `bws`, and must
# exit 1 with its named error when none does:
#
#   1. BWS_ACCESS_TOKEN already in the environment.
#   2. ~/.config/bws/access-token present (exported by the wrapper).
#   3. A bare `bws` fails, then ~/tools/bws-env.sh is sourced and
#      `bws` works.
#   4. No branch works: exit 1, the "no working Bitwarden" error,
#      and the drain step is never exec'd.
#   5. Precedence: a good env token wins over a bad config file
#      (the file is only read when the env var is unset).
#
# `bws` is stubbed: it only works when BWS_ACCESS_TOKEN is exactly
# the stub token, and it serves the two secret names the wrapper
# fetches with fixed stub values. The drain entry point
# (../bin/drain_queue.py relative to the wrapper) is a stub that
# records the environment it was exec'd with. No real credential
# appears anywhere in this test.
#
# The wrapper is invoked through `env -i` with a controlled HOME
# and PATH: some hosts inject credential helpers into interactive
# shells (exported functions / BASH_ENV) that would otherwise
# "helpfully" supply a real token and defeat the stub.
set -uo pipefail

here="$(dirname -- "$(realpath -- "$0")")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/systemd" "$work/bin" "$work/stubbin"
cp "$here/drain-wrapper.sh" "$work/systemd/drain-wrapper.sh"
chmod +x "$work/systemd/drain-wrapper.sh"

STUB_TOKEN="stub-good-token"
STUB_URL="https://worker.example.invalid"
STUB_BEARER="stub-bearer-value"

cat >"$work/bin/drain_queue.py" <<'STUB'
import os

with open(os.environ["DRAIN_MARKER"], "w") as fh:
    fh.write("url=" + os.environ.get("GHB_WORKER_URL", "") + "\n")
    fh.write("bearer=" + os.environ.get("GHB_WORKER_BEARER", "") + "\n")
    fh.write("token=" + os.environ.get("BWS_ACCESS_TOKEN", "") + "\n")
STUB

cat >"$work/stubbin/bws" <<STUB
#!/usr/bin/env bash
# Stub bws: works only with the stub token in the environment.
[ "\${BWS_ACCESS_TOKEN:-}" = "$STUB_TOKEN" ] || exit 1
if [ "\$1 \$2" = "secret list" ]; then
  printf '[{"key":"github-app.backup-worker-url","id":"id-url"},{"key":"github-app.backup-worker-bearer","id":"id-bearer"}]\n'
elif [ "\$1 \$2" = "secret get" ] && [ "\$3" = "id-url" ]; then
  printf '{"value":"$STUB_URL"}\n'
elif [ "\$1 \$2" = "secret get" ] && [ "\$3" = "id-bearer" ]; then
  printf '{"value":"$STUB_BEARER"}\n'
else
  exit 2
fi
STUB
chmod +x "$work/stubbin/bws"

fail=0
check() { # name condition...
  local name="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "PASS: $name"
  else
    echo "FAIL: $name" >&2
    fail=1
  fi
}

# run_wrapper <name> <home> [token] — runs the wrapper in a clean
# environment; stdout/stderr land in $work/out-<name>/err-<name>,
# the drain marker in $work/marker-<name>. Prints the exit code.
run_wrapper() {
  local name="$1" home="$2" token="${3:-}"
  local -a env_args=(
    "HOME=$home"
    "PATH=$work/stubbin:$PATH"
    "DRAIN_MARKER=$work/marker-$name"
  )
  [ -n "$token" ] && env_args+=("BWS_ACCESS_TOKEN=$token")
  rm -f "$work/marker-$name"
  env -i "${env_args[@]}" bash "$work/systemd/drain-wrapper.sh" \
    >"$work/out-$name" 2>"$work/err-$name"
  echo $?
}

marker_has() { # name key value
  grep -qx "$2=$3" "$work/marker-$1" 2>/dev/null
}

# 1. Token already in the environment.
home="$work/home-env-token"
mkdir -p "$home"
rc="$(run_wrapper env-token "$home" "$STUB_TOKEN")"
check "env token: wrapper exits 0" test "$rc" -eq 0
check "env token: drain received the stub URL" marker_has env-token url "$STUB_URL"
check "env token: drain received the stub bearer" marker_has env-token bearer "$STUB_BEARER"

# 2. Token only in ~/.config/bws/access-token.
home="$work/home-config-file"
mkdir -p "$home/.config/bws"
printf '%s' "$STUB_TOKEN" >"$home/.config/bws/access-token"
rc="$(run_wrapper config-file "$home")"
check "config file: wrapper exits 0" test "$rc" -eq 0
check "config file: drain received the stub URL" marker_has config-file url "$STUB_URL"
check "config file: exported token came from the file" marker_has config-file token "$STUB_TOKEN"

# 3. Bare bws fails; ~/tools/bws-env.sh supplies the token.
home="$work/home-env-file"
mkdir -p "$home/tools"
printf 'export BWS_ACCESS_TOKEN=%s\n' "$STUB_TOKEN" >"$home/tools/bws-env.sh"
rc="$(run_wrapper env-file "$home")"
check "bws-env fallback: wrapper exits 0" test "$rc" -eq 0
check "bws-env fallback: drain received the stub URL" marker_has env-file url "$STUB_URL"

# 4. No credential branch works.
home="$work/home-no-cred"
mkdir -p "$home"
rc="$(run_wrapper no-cred "$home")"
check "no credential: wrapper exits 1" test "$rc" -eq 1
check "no credential: error names the failure" grep -q "no working Bitwarden (bws) credential" "$work/err-no-cred"
check "no credential: drain never ran" test ! -e "$work/marker-no-cred"

# 5. Precedence: a good env token beats a bad config file.
home="$work/home-precedence"
mkdir -p "$home/.config/bws"
printf '%s' "stub-WRONG-token" >"$home/.config/bws/access-token"
rc="$(run_wrapper precedence "$home" "$STUB_TOKEN")"
check "precedence: wrapper exits 0 with the env token" test "$rc" -eq 0
check "precedence: drain saw the env token, not the file's" marker_has precedence token "$STUB_TOKEN"

if [ "$fail" -eq 0 ]; then
  echo "drain-wrapper.test.sh: all checks passed"
fi
exit "$fail"
