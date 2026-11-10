#!/usr/bin/env bash
# Snapshot agent credentials and config into a dated directory, on change.
#
# Deliberately dumb: no restic, no repository, no password. Each run copies the
# current credential/config set into
#
#   ~/.local/share/agent-backups/YYYY-mm-ddTHH-MM-SS/
#
# and then compares it against the most recent previous snapshot. If nothing
# changed, the new directory is removed, so the history only ever grows when
# something actually differs. A `latest` symlink points at the newest snapshot.
#
# Scope: an inventory of this machine found the irreplaceable material is
# kilobytes (auth.json files, API keys, t3 settings.json), while
# ~/.antigravity-ide and ~/.config/Antigravity IDE alone are ~4G of regenerable
# cache. Only the small, irreplaceable set is copied.
#
# Permissions: the backup root is created 0700 and files keep their modes, so
# 0600 secrets stay 0600. Note these copies are PLAINTEXT on local disk -- this
# protects against accidental deletion and bad edits, not against disk theft.

set -euo pipefail
export LC_ALL=C

ROOT="${AGENT_BACKUP_DIR:-$HOME/.local/share/agent-backups}"
mkdir -p "$ROOT"
chmod 700 "$ROOT"

BASE_STAMP="$(date -u +%Y-%m-%dT%H-%M-%S)"
STAMP="$BASE_STAMP"
DEST="$ROOT/$STAMP"

# Reserve the destination atomically before copying. Two invocations can
# receive the same second-resolution timestamp; only one may own the bare
# timestamp, and later invocations get deterministic suffixes. The no-change
# cleanup below is then limited to a directory this invocation created.
suffix=0
while ! mkdir -m 700 -- "$DEST" 2>/dev/null; do
  if [[ ! -e ${DEST} ]] && [[ ! -L ${DEST} ]]; then
    echo "agent-backup: cannot create snapshot directory ${DEST}" >&2
    exit 1
  fi
  suffix=$((suffix + 1))
  STAMP="${BASE_STAMP}-$(printf '%010d' "${suffix}")"
  DEST="$ROOT/$STAMP"
done

# Small, irreplaceable things only. Missing paths are skipped silently -- not
# every agent is signed in on every machine.
CANDIDATES=(
  "$HOME/.local/share/opencode/auth.json"
  "$HOME/.config/opencode/auth.json"
  "$HOME/.config/opencode/opencode.jsonc"
  "$HOME/.config/opencode/opencode.json"
  "$HOME/.grok/auth.json"
  "$HOME/.local/share/t3code/userdata/settings.json"
  "$HOME/.local/share/t3code/userdata/keybindings.json"
  "$HOME/.config/openchamber/ui-password"
  "$HOME/.config/openchamber/classifier-endpoint.json"
  "$HOME/.config/openchamber/classification.json"
  "$HOME/.config/cloudflare/account-id"
  "$HOME/.config/cloudflare/gateway-id"
  "$HOME/.config/cloudflare/ai-inference-token"
  "$HOME/.claude/settings.json"
  "$HOME/.claude/.credentials.json"
  "$HOME/.claude.json"
  "$HOME/.codex/config.toml"
  "$HOME/.codex/auth.json"
)

copied=0
for src in "${CANDIDATES[@]}"; do
  [ -f "$src" ] || continue
  rel="${src#"$HOME"/}"
  mkdir -p "$DEST/$(dirname "$rel")"
  cp -p "$src" "$DEST/$rel"
  copied=$((copied + 1))
done

if [ "$copied" -eq 0 ]; then
  rmdir "$DEST" 2>/dev/null || true
  echo "agent-backup: nothing present to snapshot." >&2
  exit 0
fi

# Drop the snapshot if it is identical to the previous one. `latest` is a
# symlink, so -type d already excludes it when looking for prior directories.
# PREV is the newest snapshot strictly before this run's STAMP: a future-dated
# directory (clock skew, manual copy) must not stand in for the true previous
# snapshot (#4602). Names are fixed-width timestamps, so byte order (LC_ALL=C)
# is chronological, same-second suffixed names (#3340) included: a bare stamp
# sorts before its suffixed siblings. The sentinel trick keeps this to
# findutils + coreutils + bash builtins only: the systemd service PATH has no
# awk/grep/sed, so no filter pipeline may assume them.
PREV=""
while IFS= read -r name; do
  if [[ $name == "$STAMP" ]]; then
    break
  fi
  PREV="$name"
done < <({
  find "$ROOT" -mindepth 1 -maxdepth 1 -type d -not -name "$STAMP" -printf '%f\n'
  printf '%s\n' "$STAMP"
} | LC_ALL=C sort -u)

if [ -n "$PREV" ] &&
  diff -rq "$ROOT/$PREV" "$DEST" >/dev/null 2>&1 &&
  diff -q \
    <(find "$ROOT/$PREV" -type f -printf '%m %P\n' | LC_ALL=C sort) \
    <(find "$DEST" -type f -printf '%m %P\n' | LC_ALL=C sort) \
    >/dev/null 2>&1; then
  rm -rf "$DEST"
  echo "agent-backup: no change since $PREV" >&2
  exit 0
fi

# Only move `latest` forward. A newer snapshot already on disk (future-dated
# or concurrent) keeps the pointer; this run must never regress it (#4602).
NEWEST="$(find "$ROOT" -mindepth 1 -maxdepth 1 -type d -not -name "$STAMP" -printf '%f\n' |
  LC_ALL=C sort | tail -n1 || true)"

if [ -z "$NEWEST" ] || [[ $STAMP > $NEWEST ]]; then
  ln -sfn "$STAMP" "$ROOT/latest"
fi
echo "agent-backup: snapshot $STAMP ($copied files)" >&2
