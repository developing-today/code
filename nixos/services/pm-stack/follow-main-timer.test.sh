#!/usr/bin/env bash
# follow-main-timer.test.sh — the laptop PM follow-main timer is active
# hourly and invokes the existing main-only rebuild service.
# Oracle: NixOS evaluates the amd timer to these exact systemd fields.
# Arguments: none.
if (($# != 0)); then
  echo "usage: follow-main-timer.test.sh (takes no arguments)" >&2
  exit 2
fi
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
timer="$(cd "$repo_root" && NIXPKGS_ALLOW_UNFREE=1 nix eval --json \
  .#nixosConfigurations.amd.config.systemd.timers.pm-stack-follow-main)"

jq -e '
  .description == "Follow main for the laptop PM stack"
  and .wantedBy == ["timers.target"]
  and .timerConfig.OnCalendar == "hourly"
  and .timerConfig.Persistent == true
  and .timerConfig.Unit == "agent-rebuild-switch.service"
' <<<"$timer" >/dev/null
grep -Fq '`pm-stack-follow-main.timer`' "$repo_root/doc/fleet-schedule-registry.md"

echo "PM follow-main timer NixOS evaluation passed."
