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

ROOT="${AGENT_BACKUP_DIR:-$HOME/.local/share/agent-backups}"
mkdir -p "$ROOT"
chmod 700 "$ROOT"

STAMP="$(date -u +%Y-%m-%dT%H-%M-%S)"
DEST="$ROOT/$STAMP"

# Small, irreplaceable things only. Missing paths are skipped silently -- not
# every agent is signed in on every machine.
CANDIDATES=(
  "$HOME/.config/jules/api-key"
  "$HOME/.local/share/opencode/auth.json"
  "$HOME/.config/opencode/auth.json"
  "$HOME/.config/opencode/opencode.jsonc"
  "$HOME/.config/opencode/opencode.json"
  "$HOME/.grok/auth.json"
  "$HOME/.local/share/t3code/userdata/settings.json"
  "$HOME/.local/share/t3code/userdata/keybindings.json"
  "$HOME/.config/openchamber/ui-password"
  "$HOME/.claude/settings.json"
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
# symlink, so exclude it when looking for the prior directory.
PREV="$(find "$ROOT" -mindepth 1 -maxdepth 1 -type d -not -name "$STAMP" -printf '%f\n' \
  | sort | tail -n1 || true)"

if [ -n "$PREV" ] && diff -rq "$ROOT/$PREV" "$DEST" >/dev/null 2>&1; then
  rm -rf "$DEST"
  echo "agent-backup: no change since $PREV" >&2
  exit 0
fi

ln -sfn "$STAMP" "$ROOT/latest"
echo "agent-backup: snapshot $STAMP ($copied files)" >&2
