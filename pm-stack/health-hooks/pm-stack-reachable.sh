#!/usr/bin/env bash
# C3 health hook: PM stack reachable. Fires on failure-to-connect.
# Usage: pm-stack-reachable.sh [--caller X] [--observed-at ISO] [--detail "..."]
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$HERE/run.py" --target pm-stack-reachable "$@"
EOF
cat > pm-stack/health-hooks/opencode-working.sh <<'EOF'
#!/usr/bin/env bash
# C3 health hook: OpenCode working. Fires on failure-to-connect.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$HERE/run.py" --target opencode-working "$@"
EOF
chmod +x pm-stack/health-hooks/*.sh pm-stack/health-hooks/run.py
ls -la pm-stack/health-hooks/