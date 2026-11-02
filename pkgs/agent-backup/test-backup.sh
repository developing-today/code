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

echo "agent-backup same-second collision regression: PASS"
