#!/usr/bin/env bash
# Test suite for laptop-state-backup.sh (code#1849 remainder).
# Drives the REAL script (no pasted copy) with a scratch HOME holding
# a fake ~/.local/state/opencode, ~/.ssh and secrets/*.sops.yaml, a
# stub bws printing fake credential JSON, and a stub restic that
# records its invocations and maintains a fake repo dir (snapshots
# file + config file). rclone is left unstubbed: the Drive leg runs
# restic against an rclone: URL, so the stub restic only records it.
# Scenarios:
#   * happy path — exit 0, one backup per backend (local + r2 +
#     drive) with --tag laptop-state-daily over exactly the present
#     sources, check + forget+prune per backend, marker written.
#   * lock skip — a second instance holding the flock prints SKIP and
#     exits 0, invoking restic zero times.
#   * nothing present — an empty HOME exits non-zero naming the
#     missing sources, invoking restic zero times.
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SELF_DIR/laptop-state-backup.sh"

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

make_home() { # $1 dir; fake opencode state + ssh + one sops secret
  local h="$1"
  mkdir -p "$h/.local/state/opencode" "$h/.ssh" "$h/secrets" "$h/.config/bws"
  echo '{"x":1}' >"$h/.local/state/opencode/service-prod.json"
  echo "fake-key" >"$h/.ssh/id_ed25519"
  echo "ciphertext" >"$h/secrets/pm-stack.sops.yaml"
  echo "fake-token" >"$h/.config/bws/access-token"
}

make_bin() { # $1 dir; stub bws + recording restic
  local b="$1" calls="$2"
  cat >"$b/bws" <<'EOF'
#!/bin/sh
printf '[{"key":"restic.repo-archive-password","value":"pw"},{"key":"cloudflare.r2-backups-access-key-id","value":"id"},{"key":"cloudflare.r2-backups-secret-access-key","value":"secret"},{"key":"cloudflare.r2-endpoint","value":"https://example.r2.test"}]'
EOF
  cat >"$b/restic" <<EOF
#!/bin/sh
echo "\$*" >> "$calls"
# repo comes from the -r flag value; only local paths get state files
repo="\$(printf '%s\\n' "\$@" | grep -A1 '^-r\$' | tail -1)"
if [ "\$1" = "-r" ]; then shift 2; fi
case "\$repo" in
  *:* ) :;; # remote URL (s3:/rclone:) — record invocation only
  *)
    case "\$1" in
      cat|init) mkdir -p "\$repo"; touch "\$repo/config";;
      backup) mkdir -p "\$repo"; echo "\$*" >> "\$repo/snapshots";;
      check|forget) :;;
    esac;;
esac
exit 0
EOF
  cat >"$b/rclone" <<'EOF'
#!/bin/sh
# never actually invoked (stub restic records the drive leg); present
# so resolve_tool finds it on PATH without a nix-store fallback.
exit 0
EOF
  chmod +x "$b/bws" "$b/restic" "$b/rclone"
}

FAILS=0
check() { # $1 description, rest is test command
  local desc="$1"
  shift
  if "$@"; then echo "ok: $desc"; else
    echo "FAIL: $desc"
    FAILS=$((FAILS + 1))
  fi
}

# --- happy path ---
H="$TMP_ROOT/home"
B="$TMP_ROOT/bin"
C="$TMP_ROOT/calls"
mkdir -p "$H" "$B"
: >"$C"
make_home "$H"
make_bin "$B" "$C"
OUT="$(HOME="$H" PATH="$B:$PATH" bash "$SCRIPT" 2>&1)"
RC=$?
check "happy path exits 0" [ "$RC" -eq 0 ]
check "OK verdict line" grep -q "OK laptop state backed up" <<<"$OUT"
check "marker written" [ -f "$H/backups/restic/.laptop-state-last-success-epoch" ]
check "local repo snapshotted" grep -q "laptop-state-daily" "$H/backups/restic/laptop-state/snapshots"
check "three backups (local+r2+drive)" [ "$(grep -c ' backup ' "$C" 2>/dev/null || echo 0)" = "3" ]
check "tag used" grep -q "laptop-state-daily" "$C"
check "opencode state backed up" grep -q "state/opencode" "$C"
check "ssh backed up" grep -q ".ssh" "$C"
check "sops secret backed up" grep -q "sops.yaml" "$C"
check "check per backend" [ "$(grep -c ' check' "$C" 2>/dev/null || echo 0)" = "3" ]
check "forget+prune per backend" [ "$(grep -c ' forget.*--prune' "$C" 2>/dev/null || echo 0)" = "3" ]

# --- lock skip ---
(
  exec 8>"$H/backups/restic/.laptop-state-backup.lock"
  flock -n 8 || exit 1
  OUT2="$(HOME="$H" PATH="$B:$PATH" bash "$SCRIPT" 2>&1)"
  check "locked run prints SKIP" grep -q "SKIP" <<<"$OUT2"
) || check "lock holder acquired" false
: >"$C"
(
  exec 8>"$H/backups/restic/.laptop-state-backup.lock"
  flock -n 8
  HOME="$H" PATH="$B:$PATH" bash "$SCRIPT" >/dev/null 2>&1
  check "locked run exits 0" [ "$?" -eq 0 ]
)
check "locked run invokes no restic" [ ! -s "$C" ]

# --- nothing present ---
E="$TMP_ROOT/empty"
BE="$TMP_ROOT/ebin"
CE="$TMP_ROOT/ecalls"
mkdir -p "$E/.config/bws" "$BE"
: >"$CE"
echo "fake-token" >"$E/.config/bws/access-token"
make_bin "$BE" "$CE"
OUT3="$(HOME="$E" PATH="$BE:$PATH" bash "$SCRIPT" 2>&1)"
check "empty HOME exits non-zero" [ "$?" -ne 0 ]
check "empty HOME names missing sources" grep -q "no source paths present" <<<"$OUT3"
check "empty HOME invokes no restic" [ ! -s "$CE" ]

if [ "$FAILS" -ne 0 ]; then
  echo "$FAILS failure(s)"
  exit 1
fi
echo "laptop-state-backup suite passed"
