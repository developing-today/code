#!/usr/bin/env bash
# Usage: restore-rehearsal.test.sh (no arguments). No Docker or network access.
# Oracle: injected failure => nonzero + resource cleanup; healthy => zero;
# invalid argv => usage exit 2 before any external command or scratch write.
set -euo pipefail
if (($#)); then
  echo "unexpected argument: $1" >&2
  exit 2
fi
SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/restore-rehearsal.sh"
ROOT=$(mktemp -d)
trap 'rm -rf "$ROOT"' EXIT
mkdir -p "$ROOT/bin"
cat >"$ROOT/bin/fake" <<'FAKE'
#!/usr/bin/env bash
set -eu
cmd=${0##*/}
echo "$cmd $*" >>"$CALLS"
case "$cmd" in
  date) case "$*" in +%F) echo 2026-11-24;; +%s) echo 100;; esac ;;
  sleep) : ;;
  tar)
    dest=${@: -1}
    mkdir -p "$dest/kaneo" "$dest/grist/docs" "$dest/pad"
    touch "$dest/kaneo/globals.sql" "$dest/kaneo/kaneo.dump" "$dest/grist/home.sqlite3" "$dest/pad/pad.db"
    [[ $SCENARIO != grist-docs-missing ]] || rmdir "$dest/grist/docs"
    ;;
  sqlite3)
    store=grist; [[ $* == *pad-check* ]] && store=pad
    [[ $SCENARIO != "$store-sql-error" ]] || exit 1
    if [[ $SCENARIO == "$store-sql-bad" ]]; then echo 'page 1 corrupt'; else echo ok; fi
    ;;
  curl)
    store=grist; [[ $* != *7778* ]] || store=pad; [[ $* != *5174* ]] || store=kaneo
    [[ $SCENARIO != "$store-http-error" ]] || { echo 000; exit 28; }
    if [[ $SCENARIO == "$store-http-bad" ]]; then echo 503; else echo 200; fi
    ;;
  docker)
    case "$*" in
      'exec rehearsal-pg pg_isready'*) [[ $SCENARIO != pg-ready ]] ;;
      'exec -i rehearsal-pg pg_restore'*) [[ $SCENARIO != restore ]] ;;
      'run '*--name\ rehearsal-pg*) [[ $SCENARIO != pg-start ]] ;;
      'run '*--name\ rehearsal-grist*) [[ $SCENARIO != grist-start ]] ;;
      'run '*--name\ rehearsal-pad*) [[ $SCENARIO != pad-start ]] ;;
      'run '*--name\ rehearsal-kaneo*) [[ $SCENARIO != kaneo-start ]] ;;
      *) : ;;
    esac
    ;;
esac
FAKE
chmod +x "$ROOT/bin/fake"
for cmd in docker sqlite3 curl tar sleep date; do ln -s fake "$ROOT/bin/$cmd"; done
failed=0
run_case() {
  local scenario=$1 expected=$2
  shift 2
  local home="$ROOT/$scenario" rc=0
  mkdir -p "$home/backups/pm-stack/2026-11-24"
  touch "$home/backups/pm-stack/2026-11-24/pm-stack-2026-11-24.tar.zst"
  : >"$home/calls"
  HOME="$home" PATH="$ROOT/bin:$PATH" CALLS="$home/calls" SCENARIO="$scenario" \
    bash "$SCRIPT" "$@" >"$home/output" 2>&1 || rc=$?
  local ok=1
  case "$expected" in
  success) [[ $rc == 0 ]] || ok=0 ;;
  failure) [[ $rc != 0 ]] || ok=0 ;;
  usage)
    [[ $rc == 2 && ! -s "$home/calls" && ! -e "$home/restore-rehearsal-6816" ]] || ok=0
    ;;
  esac
  if [[ $expected != usage ]]; then
    grep -q '^docker rm -f rehearsal-pg rehearsal-grist rehearsal-pad rehearsal-kaneo$' "$home/calls" || ok=0
    grep -q '^docker network rm rehearsal-net$' "$home/calls" || ok=0
  fi
  if [[ $ok == 1 ]]; then echo "PASS $scenario"; else
    echo "FAIL $scenario (exit $rc; expected $expected)"
    cat "$home/output"
    failed=$((failed + 1))
  fi
}
run_case healthy success 2026-11-24
run_case default-day success
for scenario in restore pg-ready pg-start grist-start pad-start kaneo-start \
  grist-docs-missing \
  grist-sql-error grist-sql-bad pad-sql-error pad-sql-bad \
  grist-http-error grist-http-bad pad-http-error pad-http-bad kaneo-http-error kaneo-http-bad; do
  run_case "$scenario" failure 2026-11-24
done
run_case unknown usage --wat
run_case excess usage 2026-11-24 extra
run_case invalid-date usage 2026-02-30
run_case invalid-month usage 2026-13-01
run_case malformed-date usage 2026-1-01
run_case empty-date usage ''
run_case invalid-leap usage 2025-02-29
echo "$failed failing cases"
[[ $failed == 0 ]]
