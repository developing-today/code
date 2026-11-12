# pm-apps-health — readiness probe for the PM apps (code#8940).
#
# Oracle: issue #8940's acceptance criteria. Each PM app is READY when
# its local HTTP endpoint answers at all (any status — these apps
# redirect or require auth at their roots; a served HTTP response is
# the readiness signal, mirroring the probe-as-oracle convention of
# opencode-watchdog). A connection failure or timeout is DARK.
#
# Failure semantics (start-period + grace): the first dark probe for
# an app only records a first-dark stamp and still exits 0 — app
# startup and boot are legitimately dark for a while. An app dark
# continuously for >= @graceSeconds@ seconds makes the probe exit 1,
# so pm-apps-health.service shows as FAILED in systemd until a probe
# passes again — that failed unit (plus this script's journal lines)
# is the unhealthy-state surface. Report-only by design (#8940): the
# probe NEVER restarts anything; container process failures are
# already restarted by the oci-containers systemd units, and a
# restart loop driven by readiness could destroy in-progress work.
#
# Placeholders @curl@, @stateDir@, @graceSeconds@ and @apps@ are
# substituted by the NixOS module (nixos/services/pm-apps/default.nix,
# via lib.replaceStrings on this file) and by pm-apps-health.test.sh
# (with a stub curl). Test hooks, harmless in production:
# PM_APPS_HEALTH_STATE_DIR / PM_APPS_HEALTH_GRACE_SECONDS /
# PM_APPS_HEALTH_NOW (epoch seconds) / PM_APPS_HEALTH_APPS override
# the baked values.
state_dir="${PM_APPS_HEALTH_STATE_DIR:-@stateDir@}"
grace="${PM_APPS_HEALTH_GRACE_SECONDS:-@graceSeconds@}"
now="${PM_APPS_HEALTH_NOW:-$(date +%s)}"
apps="${PM_APPS_HEALTH_APPS:-@apps@}"
mkdir -p "$state_dir"
failures=0
for app in $apps; do
  name="${app%%=*}"
  url="${app#*=}"
  stamp="$state_dir/first-dark-$name"
  # -s: quiet; no -f: any HTTP status counts as ready (see header);
  # -m 5: a hung endpoint is dark, not a hung probe (probe timeout).
  if @curl@ -s -m 5 -o /dev/null "$url"; then
    rm -f "$stamp"
    echo "pm-apps-health: $name ready ($url)"
  else
    if [ ! -f "$stamp" ]; then
      echo "$now" >"$stamp"
      echo "pm-apps-health: $name dark, grace started ($url)"
    else
      first="$(cat "$stamp")"
      age=$((now - first))
      if [ "$age" -ge "$grace" ]; then
        echo "pm-apps-health: $name UNHEALTHY - dark for ${age}s (>= ${grace}s grace) at $url" >&2
        failures=$((failures + 1))
      else
        echo "pm-apps-health: $name dark for ${age}s (grace ${grace}s, $url)"
      fi
    fi
  fi
done
if [ "$failures" -gt 0 ]; then
  exit 1
fi
exit 0
