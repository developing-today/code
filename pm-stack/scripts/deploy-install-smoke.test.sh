#!/usr/bin/env bash
# Safely smoke render/install shell paths using temporary copies, isolated
# HOME directories, fake SOPS input and guarded command stubs.
# Run from anywhere: bash pm-stack/scripts/deploy-install-smoke.test.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PM_STACK_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BASH_BIN="${BASH:-$(command -v bash)}"
REAL_RM="$(command -v rm)"
REAL_INSTALL="$(command -v install)"
REAL_CP="$(command -v cp)"
SYSTEM_PATH="/run/current-system/sw/bin:/usr/bin:/bin"
export PATH="$SYSTEM_PATH"
export PM_TEST_REAL_RM="$REAL_RM" PM_TEST_REAL_INSTALL="$REAL_INSTALL" PM_TEST_REAL_CP="$REAL_CP"

for command_name in jq chmod cp mkdir; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "FAIL - required command not found: $command_name"
    exit 1
  fi
done

TMP_BASE="${TMPDIR:-/tmp/opencode}"
mkdir -p "$TMP_BASE"
TMP="$(mktemp -d "$TMP_BASE/deploy-install-smoke.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
STUB_BIN="$TMP/stubs"
mkdir -p "$STUB_BIN"
FAILURES=0
pass() { printf 'ok - %s\n' "$1"; }
fail() { printf 'FAIL - %s\n' "$1"; FAILURES=$((FAILURES + 1)); }
contains() { grep -Fq -- "$2" "$1"; }

cat >"$STUB_BIN/sops" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
mode=decrypt
key=''
input=''
while (($#)); do
  case "$1" in
    --decrypt) mode=decrypt ;;
    -d) mode=extract ;;
    --extract) key="$2"; shift ;;
    *) input="$1" ;;
  esac
  shift
done
if [[ ! -f "$input" || ! "$input" -ef "${PM_TEST_SECRETS:?}" ]]; then
  echo "refusing non-fixture SOPS input: $input" >&2
  exit 91
fi
if [[ -n ${PM_TEST_SOPS_LOG:-} ]]; then printf '%s %s\n' "$mode" "${key:-all}" >>"$PM_TEST_SOPS_LOG"; fi
if [[ $mode == extract ]]; then
  case "$key" in
    *kaneo_admin_email*) printf '%s\n' 'smoke-admin@example.invalid' ;;
    *kaneo_admin_password*) printf '%s\n' 'fake-kaneo-password' ;;
    *grist_api_key*) printf '%s\n' 'fake-grist-key' ;;
    *) echo "unexpected extract key: $key" >&2; exit 92 ;;
  esac
else
  cat <<'YAML'
postgres_password: fake-postgres-password
auth_secret: fake-auth-secret
notification_secret_encryption_key: fake-notification-key
github_app_id: 12345
github_app_name: smoke-app
github_webhook_secret: fake-webhook-secret
github_private_key_base64: ZmFrZS1wcml2YXRlLWtleQ==
cloudflare_tunnel_token: fake-tunnel-token
pad_api_token: fake-pad-token
grist_session_secret: fake-grist-session
grist_default_email: smoke-grist@example.invalid
YAML
fi
STUB

cat >"$STUB_BIN/curl" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
url=''
for arg in "$@"; do
  case "$arg" in http://*|https://*) url="$arg" ;; esac
done
printf 'curl %s\n' "$url" >>"${PM_TEST_COMMAND_LOG:?}"
case "$url" in
  http://127.0.0.1:4870/api/health|http://100.64.20.21:4870/api/health)
    printf '{"version":"%s"}\n' "${PM_TEST_BUILD_VERSION:-smoke-build-v1}"
    ;;
  http://127.0.0.1:6056/)
    printf 'fake ready-for-agent UI\n'
    ;;
  *) echo "blocked unexpected curl URL: $url" >&2; exit 93 ;;
esac
STUB

cat >"$STUB_BIN/docker" <<'STUB'
#!/usr/bin/env bash
printf 'docker %s\n' "$*" >>"${PM_TEST_COMMAND_LOG:?}"
echo 'docker command is blocked in smoke tests' >&2
exit 94
STUB

cat >"$STUB_BIN/systemctl" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'systemctl %s\n' "$*" >>"${PM_TEST_COMMAND_LOG:?}"
case " $* " in
  *' is-active '*|*' status '*) printf 'active\n' ;;
esac
exit 0
STUB

cat >"$STUB_BIN/bun" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'bun %s\n' "$*" >>"${PM_TEST_COMMAND_LOG:?}"
if [[ ${1:-} == install ]]; then
  # Package changes are explicitly prohibited by this smoke test.
  exit 0
fi
if [[ ${1:-} == -e ]]; then
  # Return the deterministic version corresponding to the fake health API.
  cat >/dev/null
  printf '%s\n' "${PM_TEST_BUILD_VERSION:-smoke-build-v1}"
  exit 0
fi
echo "blocked unexpected bun invocation: $*" >&2
exit 95
STUB

cat >"$STUB_BIN/npm" <<'STUB'
#!/usr/bin/env bash
printf 'npm %s\n' "$*" >>"${PM_TEST_COMMAND_LOG:?}"
echo 'npm package changes are blocked in smoke tests' >&2
exit 96
STUB

cat >"$STUB_BIN/git" <<'STUB'
#!/usr/bin/env bash
printf 'git %s\n' "$*" >>"${PM_TEST_COMMAND_LOG:?}"
if [[ ${*: -2} == 'rev-parse HEAD' ]]; then
  printf '%s\n' "${PM_TEST_BUILD_VERSION:-smoke-build-v1}"
  exit 0
fi
echo "blocked unexpected git invocation: $*" >&2
exit 97
STUB

cat >"$STUB_BIN/tailscale" <<'STUB'
#!/usr/bin/env bash
printf 'tailscale %s\n' "$*" >>"${PM_TEST_COMMAND_LOG:?}"
printf '100.64.20.21\n'
STUB

cat >"$STUB_BIN/sleep" <<'STUB'
#!/usr/bin/env bash
printf 'sleep %s\n' "$*" >>"${PM_TEST_COMMAND_LOG:?}"
exit 0
STUB

cat >"$STUB_BIN/rm" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
for arg in "$@"; do
  case "$arg" in
    -*) ;;
    "$HOME"/*|"${PM_TEST_ROOT:?}"/*) ;;
    *) echo "blocked rm outside isolated HOME/temp root: $arg" >&2; exit 98 ;;
  esac
done
exec "${PM_TEST_REAL_RM:?}" "$@"
STUB

cat >"$STUB_BIN/install" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
target=''
for arg in "$@"; do [[ $arg == -* ]] || target="$arg"; done
case "$target" in
  "$HOME"/*) ;;
  *) echo "blocked install outside isolated HOME: $target" >&2; exit 99 ;;
esac
exec "${PM_TEST_REAL_INSTALL:?}" "$@"
STUB

cat >"$STUB_BIN/cp" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
target=''
for arg in "$@"; do [[ $arg == -* ]] || target="$arg"; done
case "$target" in
  "$HOME"/*) ;;
  *) echo "blocked cp outside isolated HOME: $target" >&2; exit 100 ;;
esac
exec "${PM_TEST_REAL_CP:?}" "$@"
STUB
chmod +x "$STUB_BIN"/*

COMMAND_LOG="$TMP/commands.log"
: >"$COMMAND_LOG"

# Render modes run from copied layouts, with one fake SOPS file shared by
# the three scripts. Docker/curl remain blocked as a safety backstop.
RENDER_ROOT="$TMP/render"
RENDER_SECRETS="$RENDER_ROOT/secrets/pm-stack.sops.yaml"
mkdir -p "$RENDER_ROOT/pm-stack/pad" "$RENDER_ROOT/pm-stack/grist" "$RENDER_ROOT/secrets"
printf '%s\n' 'fake encrypted render fixture' >"$RENDER_SECRETS"
cp "$PM_STACK_DIR/deploy.sh" "$RENDER_ROOT/pm-stack/deploy.sh"
cp "$PM_STACK_DIR/pad/deploy.sh" "$RENDER_ROOT/pm-stack/pad/deploy.sh"
cp "$PM_STACK_DIR/grist/deploy.sh" "$RENDER_ROOT/pm-stack/grist/deploy.sh"

run_render() {
  local script="$1" home="$2" log="$3"
  mkdir -p "$home"
  env HOME="$home" PATH="$STUB_BIN:$SYSTEM_PATH" \
    PM_TEST_ROOT="$TMP" PM_TEST_SECRETS="$RENDER_SECRETS" \
    PM_TEST_COMMAND_LOG="$COMMAND_LOG" PM_TEST_SOPS_LOG="$log" \
    PM_STACK_TAILNET_IP=100.64.20.21 \
    "$BASH_BIN" "$script" render >"$log.out" 2>&1
}

if run_render "$RENDER_ROOT/pm-stack/deploy.sh" "$TMP/render-home-pm" "$TMP/pm-sops.log"; then
  pass 'pm-stack deploy render mode runs in a temporary copy'
else
  cat "$TMP/pm-sops.log.out" 2>/dev/null || true
  fail 'pm-stack deploy render mode runs in a temporary copy'
fi
if contains "$RENDER_ROOT/pm-stack/.env" 'POSTGRES_PASSWORD=fake-postgres-password' &&
   contains "$RENDER_ROOT/pm-stack/.env" 'DISABLE_REGISTRATION=true' &&
   contains "$RENDER_ROOT/pm-stack/tunnel.env" 'TUNNEL_TOKEN=fake-tunnel-token'; then
  pass 'pm-stack render emits expected fake values'
else
  fail 'pm-stack render emits expected fake values'
fi
if [[ $(stat -c '%a' "$RENDER_ROOT/pm-stack/.env") == 600 &&
      $(stat -c '%a' "$RENDER_ROOT/pm-stack/tunnel.env") == 600 ]]; then
  pass 'pm-stack render files have mode 600'
else
  fail 'pm-stack render files have mode 600'
fi

if run_render "$RENDER_ROOT/pm-stack/pad/deploy.sh" "$TMP/render-home-pad" "$TMP/pad-sops.log"; then
  pass 'Pad deploy render mode runs in a temporary copy'
else
  cat "$TMP/pad-sops.log.out" 2>/dev/null || true
  fail 'Pad deploy render mode runs in a temporary copy'
fi
if contains "$RENDER_ROOT/pm-stack/pad/.env" 'PM_STACK_TAILNET_IP=100.64.20.21' &&
   contains "$RENDER_ROOT/pm-stack/pad/.env" 'Pad needs no server-env secrets'; then
  pass 'Pad render emits expected temporary configuration'
else
  fail 'Pad render emits expected temporary configuration'
fi
if [[ $(stat -c '%a' "$RENDER_ROOT/pm-stack/pad/.env") == 600 ]]; then pass 'Pad render file has mode 600'; else fail 'Pad render file has mode 600'; fi

if run_render "$RENDER_ROOT/pm-stack/grist/deploy.sh" "$TMP/render-home-grist" "$TMP/grist-sops.log"; then
  pass 'Grist deploy render mode runs in a temporary copy'
else
  cat "$TMP/grist-sops.log.out" 2>/dev/null || true
  fail 'Grist deploy render mode runs in a temporary copy'
fi
if contains "$RENDER_ROOT/pm-stack/grist/.env" 'GRIST_SESSION_SECRET=fake-grist-session' &&
   contains "$RENDER_ROOT/pm-stack/grist/.env" 'GRIST_DEFAULT_EMAIL=smoke-grist@example.invalid' &&
   contains "$RENDER_ROOT/pm-stack/grist/.env" 'PM_STACK_TAILNET_IP=100.64.20.21'; then
  pass 'Grist render emits expected fake values'
else
  fail 'Grist render emits expected fake values'
fi
if [[ $(stat -c '%a' "$RENDER_ROOT/pm-stack/grist/.env") == 600 ]]; then pass 'Grist render file has mode 600'; else fail 'Grist render file has mode 600'; fi
if ! grep -Fq 'docker ' "$COMMAND_LOG"; then pass 'render modes invoke no Docker commands'; else fail 'render modes invoke no Docker commands'; fi

# Syntax-check originals without executing them in the checkout/host.
AGG_INSTALL="$PM_STACK_DIR/aggregator/install.sh"
READY_INSTALL="$PM_STACK_DIR/ready-for-agent/install.sh"
if "$BASH_BIN" -n "$AGG_INSTALL"; then pass 'aggregator/install.sh syntax check'; else fail 'aggregator/install.sh syntax check'; fi
if "$BASH_BIN" -n "$READY_INSTALL"; then pass 'ready-for-agent/install.sh syntax check'; else fail 'ready-for-agent/install.sh syntax check'; fi

# Build a minimal aggregator package copy so install.sh's hash/copy logic
# runs entirely inside a disposable HOME. Side-effecting tools are stubbed.
AGG_SRC="$TMP/aggregator-src"
AGG_HOME="$TMP/aggregator-home"
mkdir -p "$AGG_SRC" "$AGG_HOME/secrets" "$AGG_HOME/pm-stack/systemd"
cp "$AGG_INSTALL" "$AGG_SRC/install.sh"
printf '%s\n' '{"name":"smoke-aggregator","version":"1.0.0"}' >"$AGG_SRC/package.json"
printf '%s\n' 'fake lockfile' >"$AGG_SRC/bun.lock"
printf '%s\n' '{"compilerOptions":{}}' >"$AGG_SRC/tsconfig.json"
printf '%s\n' '{"projects":[]}' >"$AGG_SRC/fleet-feed.json"
mkdir -p "$AGG_SRC/src" "$AGG_SRC/test" "$AGG_SRC/scripts" "$AGG_SRC/opencode-tools" "$AGG_SRC/systemd"
printf '%s\n' '// fixture' >"$AGG_SRC/src/smoke.ts"
printf '%s\n' '// fixture' >"$AGG_SRC/test/smoke.test.ts"
printf '%s\n' '# fixture' >"$AGG_SRC/scripts/smoke.sh"
printf '%s\n' '// fixture' >"$AGG_SRC/opencode-tools/smoke.ts"
printf '%s\n' '[Service]' >"$AGG_SRC/systemd/quota-gatherer.service"
printf '%s\n' '[Timer]' >"$AGG_SRC/systemd/quota-gatherer.timer"
printf '%s\n' '[Service]' >"$AGG_HOME/pm-stack/systemd/pm-aggregator.service"
printf '%s\n' 'fake encrypted aggregator fixture' >"$AGG_HOME/secrets/pm-stack.sops.yaml"
AGG_SOPS_LOG="$TMP/aggregator-sops.log"
AGG_OUTPUT="$TMP/aggregator-install.out"
if env HOME="$AGG_HOME" PATH="$STUB_BIN:$SYSTEM_PATH" \
     PM_TEST_ROOT="$TMP" PM_TEST_SECRETS="$AGG_HOME/secrets/pm-stack.sops.yaml" \
     PM_TEST_COMMAND_LOG="$COMMAND_LOG" PM_TEST_SOPS_LOG="$AGG_SOPS_LOG" \
     PM_TEST_BUILD_VERSION=smoke-build-v1 TAILNET_IP=100.64.20.21 \
     "$BASH_BIN" "$AGG_SRC/install.sh" >"$AGG_OUTPUT" 2>&1; then
  pass 'aggregator install smoke completes using temporary HOME and stubs'
else
  cat "$AGG_OUTPUT"
  fail 'aggregator install smoke completes using temporary HOME and stubs'
fi
if contains "$AGG_HOME/pm-stack/aggregator/.env" 'KANEO_ADMIN_EMAIL=smoke-admin@example.invalid' &&
   contains "$AGG_HOME/pm-stack/aggregator/.env" 'GRIST_API_KEY=fake-grist-key' &&
   contains "$AGG_HOME/pm-stack/aggregator/.env" 'PM_AGG_BUILD_VERSION=smoke-build-v1'; then
  pass 'aggregator renders environment from stubbed SOPS fields'
else
  fail 'aggregator renders environment from stubbed SOPS fields'
fi
if [[ -f "$AGG_HOME/.config/systemd/user/pm-aggregator.service" &&
      -f "$AGG_HOME/.config/systemd/user/quota-gatherer.service" &&
      -f "$AGG_HOME/pm-stack/aggregator/.deploy-tree-hash" ]]; then
  pass 'aggregator copies package and unit files only under temporary HOME'
else
  fail 'aggregator copies package and unit files only under temporary HOME'
fi
if contains "$AGG_OUTPUT" 'health version verified: smoke-build-v1'; then
  pass 'aggregator health-version gate accepts the stubbed matching version'
else
  fail 'aggregator health-version gate accepts the stubbed matching version'
fi

# Pre-seed a fake pinned executable, avoiding npm. The npm stub fails
# closed if the installer ever attempts a package change.
READY_ROOT="$TMP/ready-src"
READY_HOME="$TMP/ready-home"
mkdir -p "$READY_ROOT/systemd" "$READY_HOME/.local/bin"
cp "$READY_INSTALL" "$READY_ROOT/install.sh"
printf '%s\n' '[Service]' >"$READY_ROOT/systemd/ready-for-agent.service"
cat >"$READY_HOME/.local/bin/ready-for-agent" <<'FAKE'
#!/usr/bin/env bash
printf 'ready-for-agent 1.1.0\n'
FAKE
chmod +x "$READY_HOME/.local/bin/ready-for-agent"
READY_OUTPUT="$TMP/ready-install.out"
if env HOME="$READY_HOME" PATH="$STUB_BIN:$SYSTEM_PATH" \
     PM_TEST_ROOT="$TMP" PM_TEST_COMMAND_LOG="$COMMAND_LOG" \
     "$BASH_BIN" "$READY_ROOT/install.sh" >"$READY_OUTPUT" 2>&1; then
  pass 'ready-for-agent install smoke completes using temporary HOME and stubs'
else
  cat "$READY_OUTPUT"
  fail 'ready-for-agent install smoke completes using temporary HOME and stubs'
fi
if [[ -f "$READY_HOME/.config/systemd/user/ready-for-agent.service" ]] &&
   contains "$READY_OUTPUT" 'ready-for-agent is up'; then
  pass 'ready-for-agent copies its unit and passes the fake UI health check'
else
  fail 'ready-for-agent copies its unit and passes the fake UI health check'
fi
if ! grep -Fq 'npm ' "$COMMAND_LOG"; then pass 'ready-for-agent smoke makes no package-manager call'; else fail 'ready-for-agent smoke makes no package-manager call'; fi

if ((FAILURES)); then
  echo 'LIMITATION: HTTP, SOPS, systemd, git, Bun execution/package install, Docker and npm are stubbed; no live service, host unit, network, or package state is exercised.'
  echo 'LIMITATION: ready-for-agent clone mode is intentionally not run because it is network-heavy.'
  printf 'deploy-install-smoke.test.sh: %d check(s) failed\n' "$FAILURES"
  exit 1
fi
echo 'LIMITATION: HTTP, SOPS, systemd, git, Bun execution/package install, Docker and npm are stubbed; no live service, host unit, network, or package state is exercised.'
echo 'LIMITATION: ready-for-agent clone mode is intentionally not run because it is network-heavy; only the non-clone install path is smoke-tested.'
echo 'deploy-install-smoke.test.sh: all checks passed'
