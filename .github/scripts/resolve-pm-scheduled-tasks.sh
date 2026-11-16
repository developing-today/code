#!/usr/bin/env bash
# Resolve pm-scheduled's explicit task input or schedule fallback.
# Prints GitHub step-output key/value lines only after the full input is
# validated, so a bad explicit list cannot partially enable dispatch.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: resolve-pm-scheduled-tasks.sh <schedule> <tasks>" >&2
  exit 2
fi

schedule="$1"
tasks_input="$2"

# Whitespace-only input means no explicit task selection. Per-token
# trimming below also handles surrounding whitespace on individual names.
tasks_input_trimmed="${tasks_input#"${tasks_input%%[![:space:]]*}"}"
tasks_input_trimmed="${tasks_input_trimmed%"${tasks_input_trimmed##*[![:space:]]}"}"

if [[ -n $tasks_input_trimmed ]]; then
  tasks=""
  remainder="$tasks_input"
  position=0

  while :; do
    ((position += 1))
    if [[ $remainder == *,* ]]; then
      token="${remainder%%,*}"
      remainder="${remainder#*,}"
      has_more=true
    else
      token="$remainder"
      has_more=false
    fi

    token="${token#"${token%%[![:space:]]*}"}"
    token="${token%"${token##*[![:space:]]}"}"
    if [[ -z $token ]]; then
      printf 'resolve-pm-scheduled-tasks: empty task token at position %d in INPUT_TASKS\n' "$position" >&2
      exit 1
    fi
    case "$token" in
    bolt | palette | suggestions) ;;
    *)
      printf "resolve-pm-scheduled-tasks: unknown task token '%s' at position %d; valid tasks: bolt, palette, suggestions\n" \
        "$token" "$position" >&2
      exit 1
      ;;
    esac

    if [[ -n $tasks ]]; then
      tasks+=,
    fi
    tasks+="$token"

    [[ $has_more == true ]] || break
  done
else
  # The quoted patterns are literal cron expressions: an unquoted '*'
  # would be interpreted as a shell glob. Empty and unknown schedules
  # intentionally select every task.
  case "$schedule" in
  # Task slots -> bolt,palette.
  "0 1 * * *") tasks="bolt,palette" ;;   # developing-today/code — 20:00 CDT (previous evening)
  "0 2 * * *") tasks="bolt,palette" ;;   # developing-today-forks/openchamber — 21:00 CDT (previous evening)
  "0 0 * * *") tasks="bolt,palette" ;;   # developing-today-forks/opencode — 19:00 CDT (previous evening)
  "0 5 * * *") tasks="bolt,palette" ;;   # dezren39/darwin-nix + dezren39/dezren39 — 00:00 CDT
  "30 13 * * *") tasks="bolt,palette" ;; # developing-today/zettel — 08:30 CDT

  # Suggestions slots -> suggestions.
  "0 13 * * *") tasks="suggestions" ;; # developing-today/code — 08:00 CDT
  "0 14 * * *") tasks="suggestions" ;; # developing-today-forks/openchamber — 09:00 CDT
  "0 15 * * *") tasks="suggestions" ;; # developing-today-forks/opencode — 10:00 CDT
  "0 16 * * *") tasks="suggestions" ;; # dezren39/darwin-nix — 11:00 CDT
  "0 17 * * *") tasks="suggestions" ;; # developing-today/zettel — 12:00 CDT
  "0 18 * * *") tasks="suggestions" ;; # dezren39/dezren39 — 13:00 CDT

  *)
    tasks="bolt,palette,suggestions"
    ;;
  esac
fi

has_bolt=false
has_palette=false
has_suggestions=false
case ",$tasks," in *",bolt,"*) has_bolt=true ;; esac
case ",$tasks," in *",palette,"*) has_palette=true ;; esac
case ",$tasks," in *",suggestions,"*) has_suggestions=true ;; esac

printf 'tasks=%s\nhas_bolt=%s\nhas_palette=%s\nhas_suggestions=%s\n' \
  "$tasks" "$has_bolt" "$has_palette" "$has_suggestions"
