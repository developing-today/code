#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "${TMP}/bin" "${TMP}/home/.claude" "${TMP}/backups/2025-12-31T23-59-59/.claude"
printf 'OLD\n' >"${TMP}/backups/2025-12-31T23-59-59/.claude/settings.json"
cat >"${TMP}/bin/date" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "${AGENT_BACKUP_TEST_STAMP:?}"
EOF
chmod +x "${TMP}/bin/date"

export HOME="${TMP}/home"
export AGENT_BACKUP_DIR="${TMP}/backups"
export AGENT_BACKUP_TEST_STAMP="2026-01-01T00-00-00"
export PATH="${TMP}/bin:${PATH}"

# The first run owns the bare timestamp and must preserve its snapshot.
printf 'NEW\n' >"${HOME}/.claude/settings.json"
bash "${SCRIPT_DIR}/backup.sh" >/dev/null 2>&1
actual="$(cat "${AGENT_BACKUP_DIR}/2026-01-01T00-00-00/.claude/settings.json")"
latest="$(readlink "${AGENT_BACKUP_DIR}/latest")"
[[ ${actual} == NEW ]]
[[ ${latest} == "2026-01-01T00-00-00" ]]

# A second run with the same timestamp gets its own directory. Even when its
# content matches the older snapshot, it must not delete the first run's data.
printf 'OLD\n' >"${HOME}/.claude/settings.json"
bash "${SCRIPT_DIR}/backup.sh" >/dev/null 2>&1
second="${AGENT_BACKUP_DIR}/2026-01-01T00-00-00-0000000001"
actual="$(cat "${second}/.claude/settings.json")"
first="$(cat "${AGENT_BACKUP_DIR}/2026-01-01T00-00-00/.claude/settings.json")"
latest="$(readlink "${AGENT_BACKUP_DIR}/latest")"
[[ ${actual} == OLD ]]
[[ ${first} == NEW ]]
[[ ${latest} == "2026-01-01T00-00-00-0000000001" ]]
[[ -e "${AGENT_BACKUP_DIR}/latest" ]]

# A later unique run identical to the previous snapshot removes only its own
# newly reserved directory and leaves latest resolving to a real snapshot.
export AGENT_BACKUP_TEST_STAMP="2026-01-01T00-00-01"
bash "${SCRIPT_DIR}/backup.sh" >/dev/null 2>&1
[[ ! -e "${AGENT_BACKUP_DIR}/2026-01-01T00-00-01" ]]
latest="$(readlink "${AGENT_BACKUP_DIR}/latest")"
[[ ${latest} == "2026-01-01T00-00-00-0000000001" ]]
[[ -e "${AGENT_BACKUP_DIR}/latest" ]]

# A mode-only change is significant: the snapshot must retain the new mode.
# The fixture's creation mode depends on the caller's umask, so save it and
# pick a transition mode that is guaranteed to differ from it.
fixture_mode="$(stat -c '%a' "${HOME}/.claude/settings.json")"
mode_test_mode=600
[[ ${fixture_mode} == 600 ]] && mode_test_mode=644
chmod "${mode_test_mode}" "${HOME}/.claude/settings.json"
export AGENT_BACKUP_TEST_STAMP="2026-01-01T00-00-02"
bash "${SCRIPT_DIR}/backup.sh" >/dev/null 2>&1
mode_snapshot="${AGENT_BACKUP_DIR}/2026-01-01T00-00-02/.claude/settings.json"
[[ -f "${mode_snapshot}" ]]
[[ "$(stat -c '%a' "${mode_snapshot}")" == "${mode_test_mode}" ]]

# With content, file set, and modes unchanged, the next snapshot still dedups.
export AGENT_BACKUP_TEST_STAMP="2026-01-01T00-00-03"
bash "${SCRIPT_DIR}/backup.sh" >/dev/null 2>&1
[[ ! -e "${AGENT_BACKUP_DIR}/2026-01-01T00-00-03" ]]
latest="$(readlink "${AGENT_BACKUP_DIR}/latest")"
[[ ${latest} == "2026-01-01T00-00-02" ]]

# Restore the fixture's original mode so the future-dated content cases
# compare files with matching modes, as intended (the seeded snapshots are
# created under the same umask as the fixture was).
chmod "${fixture_mode}" "${HOME}/.claude/settings.json"

# A future-dated snapshot must neither become the dedup baseline nor lose the
# `latest` pointer (#4602). Seed a true previous snapshot and a newer
# future-dated one, with `latest` already on the future snapshot.
mkdir -p "${AGENT_BACKUP_DIR}/2026-01-02T00-00-00/.claude" \
  "${AGENT_BACKUP_DIR}/2027-01-01T00-00-00/.claude"
printf 'OLD\n' >"${AGENT_BACKUP_DIR}/2026-01-02T00-00-00/.claude/settings.json"
printf 'FUTURE\n' >"${AGENT_BACKUP_DIR}/2027-01-01T00-00-00/.claude/settings.json"
ln -sfn "2027-01-01T00-00-00" "${AGENT_BACKUP_DIR}/latest"

# Content matching the true previous snapshot is a duplicate even though a
# newer snapshot exists: the new directory is removed and `latest` is kept.
export AGENT_BACKUP_TEST_STAMP="2026-06-01T00-00-00"
printf 'OLD\n' >"${HOME}/.claude/settings.json"
bash "${SCRIPT_DIR}/backup.sh" >/dev/null 2>&1
[[ ! -e "${AGENT_BACKUP_DIR}/2026-06-01T00-00-00" ]]
latest="$(readlink "${AGENT_BACKUP_DIR}/latest")"
[[ ${latest} == "2027-01-01T00-00-00" ]]

# Content matching the future snapshot but differing from the true previous
# one is a real transition: the new snapshot is kept, and `latest` still does
# not regress to it.
export AGENT_BACKUP_TEST_STAMP="2026-06-02T00-00-00"
printf 'FUTURE\n' >"${HOME}/.claude/settings.json"
bash "${SCRIPT_DIR}/backup.sh" >/dev/null 2>&1
actual="$(cat "${AGENT_BACKUP_DIR}/2026-06-02T00-00-00/.claude/settings.json")"
[[ ${actual} == FUTURE ]]
latest="$(readlink "${AGENT_BACKUP_DIR}/latest")"
[[ ${latest} == "2027-01-01T00-00-00" ]]

# A run newer than everything on disk moves `latest` forward again.
export AGENT_BACKUP_TEST_STAMP="2028-01-01T00-00-00"
printf 'NEWER\n' >"${HOME}/.claude/settings.json"
bash "${SCRIPT_DIR}/backup.sh" >/dev/null 2>&1
latest="$(readlink "${AGENT_BACKUP_DIR}/latest")"
[[ ${latest} == "2028-01-01T00-00-00" ]]

echo "agent-backup same-second collision regression: PASS"
echo "agent-backup future-dated PREV/latest regression: PASS"
echo "agent-backup mode-aware dedup regression: PASS"
