#!/usr/bin/env bash
# Safe, stateful smoke tests for the Grist and Pad seed scripts.
# Run from anywhere: bash pm-stack/scripts/seed-smoke.test.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PM_STACK_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BASH_BIN="${BASH:-$(command -v bash)}"
SYSTEM_PATH="/run/current-system/sw/bin:/usr/bin:/bin"
export PATH="$SYSTEM_PATH"

if ! command -v jq >/dev/null 2>&1; then
  echo 'FAIL - jq is required by the seed scripts but was not found in PATH'
  exit 1
fi

TMP_BASE="${TMPDIR:-/tmp/opencode}"
mkdir -p "$TMP_BASE"
TMP="$(mktemp -d "$TMP_BASE/seed-smoke.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
STUB_BIN="$TMP/bin"
HOME_DIR="$TMP/home"
SECRETS_FILE="$TMP/secrets/fake.sops.yaml"
mkdir -p "$STUB_BIN" "$HOME_DIR/.config/sops/age" "$(dirname "$SECRETS_FILE")"
printf '%s\n' 'test fixture only; never decrypted by real sops' >"$SECRETS_FILE"
printf '%s\n' 'fake age key location' >"$HOME_DIR/.config/sops/age/keys.txt"

FAILURES=0
pass() { printf 'ok - %s\n' "$1"; }
fail() { printf 'FAIL - %s\n' "$1"; FAILURES=$((FAILURES + 1)); }
contains() { grep -Fq -- "$2" "$1"; }

cat >"$STUB_BIN/sops" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
input=''
mode=decrypt
key=''
while (($#)); do
  case "$1" in
    --decrypt) mode=decrypt ;;
    -d) mode=extract ;;
    --extract) key="$2"; shift ;;
    *) input="$1" ;;
  esac
  shift
done
if [[ "$input" != "${PM_TEST_SECRETS:?}" || ! -f "$input" ]]; then
  echo "refusing non-fixture SOPS input: $input" >&2
  exit 91
fi
if [[ $mode == extract ]]; then
  echo 'unexpected extract request in seed test' >&2
  exit 92
fi
cat <<'YAML'
grist_api_key: fake-grist-key
pad_api_token: fake-pad-token
YAML
STUB

cat >"$STUB_BIN/curl-grist" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
method=GET
url=''
body=''
while (($#)); do
  case "$1" in
    -X) method="$2"; shift ;;
    -H) shift ;;
    -d)
      if [[ "$2" == @- ]]; then body="$(cat)"; else body="$2"; fi
      shift
      ;;
    --fail|--silent) ;;
    http://*) url="$1" ;;
  esac
  shift
done
case "$url" in
  http://127.0.0.1:8484*) path="${url#http://127.0.0.1:8484}" ;;
  *) echo "blocked unexpected Grist URL: $url" >&2; exit 93 ;;
esac
log="${SMOKE_CURL_LOG:?}"
state="${SMOKE_STATE:?}"
phase="${SMOKE_PHASE:-unknown}"
event=''
if [[ $method == POST && $path == /api/docs/42/apply ]]; then
  action="$(printf '%s' "$body" | jq -r '.[0][0]')"
  event="action=$action"
  case "$action" in
    AddTable) : >"$state/table" ;;
    BulkAddRecord) : >"$state/rows" ;;
    *) echo "unexpected Grist apply action: $action" >&2; exit 94 ;;
  esac
fi
printf 'phase=%s method=%s path=%s %s\n' "$phase" "$method" "$path" "$event" >>"$log"
if [[ $method == POST ]]; then
  case "$path" in
    /api/orgs/1/workspaces) : >"$state/workspace"; printf '11\n' ;;
    /api/workspaces/11/docs) : >"$state/document"; printf '42\n' ;;
    /api/docs/42/apply) printf '{}\n' ;;
    *) echo "blocked unexpected Grist POST: $path" >&2; exit 95 ;;
  esac
  exit 0
fi
case "$path" in
  /api/orgs) printf '[{"id":1}]\n' ;;
  /api/orgs/1/workspaces)
    if [[ -f "$state/workspace" ]]; then
      if [[ -f "$state/document" ]]; then
        printf '[{"id":11,"name":"Roadmap & registers","docs":[{"id":42,"name":"Program roadmap"}]}]\n'
      else
        printf '[{"id":11,"name":"Roadmap & registers","docs":[]}]\n'
      fi
    else
      printf '[]\n'
    fi
    ;;
  /api/docs/42/tables)
    if [[ -f "$state/table" ]]; then printf '{"tables":[{"id":"Roadmap"}]}\n'; else printf '{"tables":[]}\n'; fi
    ;;
  /api/docs/42/tables/Roadmap/records)
    if [[ -f "$state/rows" ]]; then printf '{"records":[{"id":1}]}\n'; else printf '{"records":[]}\n'; fi
    ;;
  *) echo "blocked unexpected Grist GET: $path" >&2; exit 96 ;;
esac
STUB

cat >"$STUB_BIN/curl-pad" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
method=GET
url=''
body=''
while (($#)); do
  case "$1" in
    -X) method="$2"; shift ;;
    -H) shift ;;
    -d)
      if [[ "$2" == @- ]]; then body="$(cat)"; else body="$2"; fi
      shift
      ;;
    --fail|--silent) ;;
    http://*) url="$1" ;;
  esac
  shift
done
case "$url" in
  http://127.0.0.1:7777/api/v1/workspaces/developing-today/collections/*/items) path="${url#http://127.0.0.1:7777/api/v1/workspaces/developing-today}" ;;
  *) echo "blocked unexpected Pad URL: $url" >&2; exit 97 ;;
esac
log="${SMOKE_CURL_LOG:?}"
state="${SMOKE_STATE:?}"
phase="${SMOKE_PHASE:-unknown}"
titles="$state/titles"
mkdir -p "$state"
if [[ $method == POST ]]; then
  title="$(printf '%s' "$body" | jq -r '.title')"
  printf '%s\n' "$title" >>"$titles"
  printf 'phase=%s method=POST path=%s title=%s\n' "$phase" "$path" "$title" >>"$log"
  printf '{}\n'
else
  printf 'phase=%s method=GET path=%s\n' "$phase" "$path" >>"$log"
  if [[ -f "$titles" ]]; then
    jq -Rsc 'split("\n") | map(select(length > 0) | {title: .})' "$titles"
  else
    printf '[]\n'
  fi
fi
STUB
chmod +x "$STUB_BIN/sops" "$STUB_BIN/curl-grist" "$STUB_BIN/curl-pad"

GRIST_STATE="$TMP/grist-state"
PAD_STATE="$TMP/pad-state"
GRIST_LOG="$TMP/grist-curl.log"
PAD_LOG="$TMP/pad-curl.log"
mkdir -p "$TMP/grist-bin" "$TMP/pad-bin" "$GRIST_STATE" "$PAD_STATE"
ln -s "$STUB_BIN/curl-grist" "$TMP/grist-bin/curl"
ln -s "$STUB_BIN/curl-pad" "$TMP/pad-bin/curl"
: >"$GRIST_LOG"
: >"$PAD_LOG"

run_grist() {
  local phase="$1" output="$2"
  env HOME="$HOME_DIR" PATH="$TMP/grist-bin:$STUB_BIN:$SYSTEM_PATH" \
    PM_STACK_SECRETS="$SECRETS_FILE" PM_TEST_SECRETS="$SECRETS_FILE" \
    SOPS_AGE_KEY_FILE="$HOME_DIR/.config/sops/age/keys.txt" \
    SMOKE_PHASE="$phase" SMOKE_STATE="$GRIST_STATE" SMOKE_CURL_LOG="$GRIST_LOG" \
    "$BASH_BIN" "$PM_STACK_DIR/grist/seed.sh" >"$output" 2>&1
}
run_pad() {
  local phase="$1" output="$2"
  env HOME="$HOME_DIR" PATH="$TMP/pad-bin:$STUB_BIN:$SYSTEM_PATH" \
    PM_STACK_SECRETS="$SECRETS_FILE" PM_TEST_SECRETS="$SECRETS_FILE" \
    SOPS_AGE_KEY_FILE="$HOME_DIR/.config/sops/age/keys.txt" \
    SMOKE_PHASE="$phase" SMOKE_STATE="$PAD_STATE" SMOKE_CURL_LOG="$PAD_LOG" \
    "$BASH_BIN" "$PM_STACK_DIR/pad/seed.sh" >"$output" 2>&1
}

if run_grist first "$TMP/grist-first.out"; then pass 'Grist first seed run completes'; else cat "$TMP/grist-first.out"; fail 'Grist first seed run completes'; fi
if run_grist second "$TMP/grist-second.out"; then pass 'Grist second seed run completes'; else cat "$TMP/grist-second.out"; fail 'Grist second seed run completes'; fi
if contains "$TMP/grist-first.out" 'seeded '; then pass 'Grist first run seeds the initially empty Roadmap'; else fail 'Grist first run seeds the initially empty Roadmap'; fi
if contains "$TMP/grist-second.out" 'skip: Roadmap already has '; then pass 'Grist second run reports existing rows'; else fail 'Grist second run reports existing rows'; fi
if contains "$GRIST_LOG" 'phase=first method=POST path=/api/docs/42/apply action=AddTable' &&
   contains "$GRIST_LOG" 'phase=first method=POST path=/api/docs/42/apply action=BulkAddRecord'; then
  pass 'Grist first run creates the table and writes rows in fake API state'
else
  fail 'Grist first run creates the table and writes rows in fake API state'
fi
if ! grep -Fq 'phase=second method=POST' "$GRIST_LOG"; then
  pass 'Grist second run performs no creates or row writes'
else
  fail 'Grist second run performs no creates or row writes'
fi

if run_pad first "$TMP/pad-first.out"; then pass 'Pad first seed run completes'; else cat "$TMP/pad-first.out"; fail 'Pad first seed run completes'; fi
if run_pad second "$TMP/pad-second.out"; then pass 'Pad second seed run completes'; else cat "$TMP/pad-second.out"; fail 'Pad second seed run completes'; fi
PAD_TITLES=(
  'Project-management stack — design & phase plan'
  'Phase 1 — as-built (Kaneo execution board)'
  'Phase 2 — plan (dispatch, planning, roadmap layers)'
)
for title in "${PAD_TITLES[@]}"; do
  if contains "$TMP/pad-second.out" "skip (exists): [plans] $title"; then
    pass "Pad second run skips: $title"
  else
    fail "Pad second run skips: $title"
  fi
done
if [[ $(grep -Fc 'phase=first method=POST' "$PAD_LOG" || true) -eq 3 ]] &&
   ! grep -Fq 'phase=second method=POST' "$PAD_LOG"; then
  pass 'Pad second run performs no creates; all three first-run titles persist'
else
  fail 'Pad second run performs no creates; all three first-run titles persist'
fi

if ((FAILURES)); then
  printf 'seed-smoke.test.sh: %d check(s) failed\n' "$FAILURES"
  exit 1
fi
echo 'LIMITATION: SOPS and HTTP are simulated; these checks verify idempotence against fake API state, not a live Grist/Pad deployment.'
echo 'seed-smoke.test.sh: all checks passed'
