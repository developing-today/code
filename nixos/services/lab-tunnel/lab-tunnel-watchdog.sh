# lab-tunnel-watchdog — restart lab-tunnel when its edge connections
# stay down (#8933, mirroring pm-tunnel's #6992 watchdog).
#
# cloudflared can stay process-alive with no live edge connections;
# its own /ready endpoint is the truth (200 = at least one live edge
# connection). Three consecutive failures (the timer runs every 2
# minutes) restart lab-tunnel; any success resets the counter.
#
# Placeholders @curl@, @systemctl@ and @readyUrl@ are substituted by
# the NixOS module (nixos/services/lab-tunnel/default.nix, via
# lib.replaceStrings on this file) and by lab-tunnel-watchdog.test.sh
# (with stub binaries) — keep them intact.
stateDir="$HOME/.local/state/lab-tunnel-watchdog"
mkdir -p "$stateDir"
countFile="$stateDir/consecutive-failures"
count=0
if [ -f "$countFile" ]; then
  count=$(cat "$countFile")
fi
if @curl@ -fsS -m 10 "@readyUrl@" >/dev/null 2>&1; then
  echo 0 > "$countFile"
  exit 0
fi
count=$((count + 1))
echo "$count" > "$countFile"
echo "lab-tunnel-watchdog: /ready failed ($count consecutive)" >&2
if [ "$count" -ge 3 ]; then
  echo 0 > "$countFile"
  exec @systemctl@ --user restart lab-tunnel.service
fi
