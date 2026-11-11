#!/usr/bin/env bash
# check-workflows.sh — generic workflow validation (#5743).
#
# The basic-tier gate for .github/workflows/** and .github/actions/**:
# mcpx.yml's actionlint leg covers only .github/workflows/mcpx*.yml
# (#1996), so workflow and composite-action changes elsewhere could
# merge with no validator at all. This script is that validator:
#
#   - workflow files: actionlint (syntax, expressions, action
#     references and their inputs), with shellcheck over run: blocks
#     at -S warning when shellcheck is available (same severity as
#     the mcpx leg).
#   - action metadata (action.yml / action.yaml): actionlint parses
#     every file it is given as a workflow, so composite-action
#     metadata gets a dedicated shape validation instead (below).
#
# Usage:
#   check-workflows.sh                lint the full repo set
#   check-workflows.sh FILE [FILE..]  lint exactly these files
#                                     (used by the contract tests)
#
# actionlint is taken from $ACTIONLINT_BIN when set, else from PATH.
# The workflow-lint workflow installs the pinned release (with
# checksum verification) before calling this script.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CALLER_DIR="$PWD"
cd "$REPO_ROOT"

ACTIONLINT="${ACTIONLINT_BIN:-actionlint}"
if ! command -v "$ACTIONLINT" >/dev/null 2>&1; then
  echo "check-workflows: actionlint not found (set ACTIONLINT_BIN or put it on PATH)" >&2
  exit 1
fi

# actionlint shellcheck integration: same severity as mcpx.yml.
if command -v shellcheck >/dev/null 2>&1; then
  SHELLCHECK_OPT=(-shellcheck "shellcheck -S warning")
else
  echo "check-workflows: shellcheck not on PATH; run: blocks are not shellchecked" >&2
  SHELLCHECK_OPT=(-shellcheck "")
fi

# validate_action_metadata FILE — shape-check one action.yml.
# Oracle: the GitHub composite/action metadata schema — a mapping
# with name + description, a runs mapping whose using is one of the
# supported runtimes, steps for composite actions, and no unknown
# top-level keys (a typo'd `run:` for `runs:` is the failure this
# catches). PyYAML gives a real parse when importable, and the full
# contract — per-runtime required fields (docker runs.image, node
# runs.main) and composite step shapes (mapping steps, each with
# run or uses, shell on run steps) — is enforced on that branch.
# The structural fallback (no PyYAML) enforces a REDUCED contract:
# top-level keys and a supported runs.using only; it does not check
# per-runtime fields or step shapes (#6871).
validate_action_metadata() {
  local file="$1"
  if [ ! -r "$file" ]; then
    # A missing or unreadable file is a read failure, not a shape
    # failure: name it as one here, before the PyYAML branch can call
    # it "invalid YAML" or the fallback can invent an empty key (#6872).
    local reason="Permission denied"
    [ -e "$file" ] || reason="No such file or directory"
    echo "$file: cannot read action file: $reason" >&2
    return 1
  fi
  if python3 -c 'import yaml' >/dev/null 2>&1; then
    python3 - "$file" <<'PYEOF'
import sys
import yaml

path = sys.argv[1]
try:
    with open(path, encoding="utf-8") as fh:
        doc = yaml.safe_load(fh)
except OSError as exc:  # cannot read the file at all: say so, plainly
    print(f"{path}: cannot read action file: {exc}", file=sys.stderr)
    sys.exit(1)
except Exception as exc:  # YAML syntax error: fail with the parser's words
    print(f"{path}: invalid YAML: {exc}", file=sys.stderr)
    sys.exit(1)

errors = []
if not isinstance(doc, dict):
    errors.append("top level is not a mapping")
else:
    allowed = {"name", "author", "description", "inputs", "outputs", "runs", "branding"}
    for key in doc:
        if key not in allowed:
            errors.append(f"unexpected top-level key {key!r}")
    for key in ("name", "description"):
        if not isinstance(doc.get(key), str) or not doc[key]:
            errors.append(f"{key!r} must be a non-empty string")
    runs = doc.get("runs")
    if not isinstance(runs, dict):
        errors.append("'runs' must be a mapping")
    else:
        using = runs.get("using")
        if using not in ("composite", "docker", "node16", "node20", "node24"):
            errors.append(f"runs.using {using!r} is not a supported runtime")
        elif using == "composite":
            steps = runs.get("steps")
            if not isinstance(steps, list) or not steps:
                errors.append("composite action must declare a non-empty runs.steps list")
            else:
                for i, step in enumerate(steps):
                    if not isinstance(step, dict):
                        errors.append(f"runs.steps[{i}] must be a mapping")
                        continue
                    has_run = isinstance(step.get("run"), str) and bool(step["run"])
                    has_uses = isinstance(step.get("uses"), str) and bool(step["uses"])
                    if not has_run and not has_uses:
                        errors.append(f"runs.steps[{i}] must declare 'run' or 'uses'")
                    elif has_run and not (
                        isinstance(step.get("shell"), str) and step["shell"]
                    ):
                        errors.append(f"runs.steps[{i}] with 'run' must declare 'shell'")
        elif using == "docker":
            if not isinstance(runs.get("image"), str) or not runs["image"]:
                errors.append("docker action must declare runs.image")
        elif using in ("node16", "node20", "node24"):
            if not isinstance(runs.get("main"), str) or not runs["main"]:
                errors.append(f"{using} action must declare runs.main as a non-empty string")
for err in errors:
    print(f"{path}: {err}", file=sys.stderr)
sys.exit(1 if errors else 0)
PYEOF
  else
    echo "check-workflows: PyYAML not importable; structural fallback for $file" >&2
    local keys using key
    keys="$(grep -oE '^[A-Za-z][A-Za-z0-9_-]*:' "$file" | tr -d ':' | sort -u)"
    local allowed=" name author description inputs outputs runs branding "
    while IFS= read -r key; do
      case "$allowed" in
        *" $key "*) ;;
        *) echo "$file: unexpected top-level key '$key'" >&2; return 1 ;;
      esac
    done <<<"$keys"
    for key in name description runs; do
      if ! grep -qx "$key" <<<"$keys"; then
        echo "$file: missing top-level key '$key'" >&2
        return 1
      fi
    done
    using="$(awk '/^runs:/{in_runs=1; next} /^[A-Za-z]/{in_runs=0} in_runs && /using:/{print; exit}' "$file")"
    case "$using" in
      *composite*|*docker*|*node16*|*node20*|*node24*) ;;
      *) echo "$file: runs.using is missing or not a supported runtime" >&2; return 1 ;;
    esac
  fi
}

classify() { # classify FILE -> workflow|action
  case "$(basename "$1")" in
    action.yml|action.yaml) echo action ;;
    *) echo workflow ;;
  esac
}

declare -a WORKFLOWS=() ACTIONS=()
if [ "$#" -gt 0 ]; then
  for f in "$@"; do
    # Explicit FILE args are the caller's paths: resolve relative ones
    # against the caller's working directory, not the repo root this
    # script cd'ed to above (#6872).
    case "$f" in
      /*) ;;
      *) f="$CALLER_DIR/$f" ;;
    esac
    if [ "$(classify "$f")" = action ]; then ACTIONS+=("$f"); else WORKFLOWS+=("$f"); fi
  done
else
  while IFS= read -r f; do WORKFLOWS+=("$f"); done < <(
    find .github/workflows -maxdepth 1 \( -name '*.yml' -o -name '*.yaml' \) | sort)
  while IFS= read -r f; do ACTIONS+=("$f"); done < <(
    find .github/actions -name 'action.yml' -o -name 'action.yaml' | sort)
fi

status=0
if [ "${#WORKFLOWS[@]}" -gt 0 ]; then
  echo "check-workflows: actionlint over ${#WORKFLOWS[@]} workflow file(s)"
  "$ACTIONLINT" "${SHELLCHECK_OPT[@]}" "${WORKFLOWS[@]}" || status=1
fi
for f in "${ACTIONS[@]}"; do
  echo "check-workflows: validating action metadata $f"
  validate_action_metadata "$f" || status=1
done
if [ "$status" -eq 0 ]; then
  echo "check-workflows: OK"
fi
exit "$status"
