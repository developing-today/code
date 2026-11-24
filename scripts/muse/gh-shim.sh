#!/bin/bash
# gh-shim.sh — `gh` routes to `fleet gh` by default (Drew, 2026-11-14).
#
# Installed as /usr/local/bin/gh (symlink) and ~/tools/bin/gh (symlink);
# init-ensure.sh repoints /usr/local/bin/gh here after every sandbox
# recycle. The pristine gh stays reachable as `gh-orig`.
#
# How it works:
#   * Normal calls exec `fleet gh "$@"`. Fleet resolves the real gh
#     through FLEET_GH_BIN, which this shim pins to the resolved
#     nix-profile gh — so fleet never PATH-resolves `gh` back into
#     this shim (no recursion). FLEET_GH_SHIM_ACTIVE is a second,
#     belt-and-braces guard: any nested invocation goes straight to
#     the real gh.
#   * Local-only informational verbs (--version, --help, -h, help,
#     and bare `gh`) exec the real gh directly: they need no token,
#     and `gh --help` must print gh's own help, not fleet's usage.
#   * Escape hatches: `gh-orig` for the pristine binary, and
#     `fleet gh --raw …` / FLEET_GH_RAW=1 for a no-rotation call
#     through fleet itself.
#   * If the fleet binary is missing or broken, the shim says so on
#     stderr and execs the real gh (fail visible, never silent).
#   * While the routes-ensure marker (~/.local/state/fleet/gh-route,
#     maintained by ~/tools/fleet-routes-ensure.sh) says the fleet
#     gh route is down, calls go straight to the real gh the same
#     way; the marker flips back when the route recovers.
set -u

REAL_GH=""
# /usr/bin/gh joined the candidates 2026-11-22: while nix is out the
# nix-profile path dangles, which left this shim with no real gh at
# all (every call exited 127); the packaged /usr/bin/gh is the
# in-process signed-in gh and is also the routes-ensure fallback.
for cand in /home/hatch/.nix-profile/bin/gh /usr/bin/gh /home/hatch/tools/bin/gh-orig; do
  if [ -x "$cand" ]; then
    REAL_GH=$(readlink -f "$cand" 2>/dev/null || echo "$cand")
    break
  fi
done
if [ -z "$REAL_GH" ] || [ ! -x "$REAL_GH" ]; then
  echo "gh-shim: cannot locate the real gh binary; giving up" >&2
  exit 127
fi

# Recursion guard: fleet (or anything under this shim) calling `gh`
# again gets the real binary.
if [ "${FLEET_GH_SHIM_ACTIVE:-}" = "1" ]; then
  exec "$REAL_GH" "$@"
fi

# Local-only informational verbs: no token, no rotation.
case "${1:-}" in
--version | --help | -h | help | "")
  exec "$REAL_GH" "$@"
  ;;
esac

# Shim-level raw passthrough (Drewry, 2026-11-24): `gh --raw ...`
# strips --raw and execs the real gh directly — the shim-side twin of
# `fleet gh --raw ...`, for callers that only have `gh` in hand.
if [ "${1:-}" = "--raw" ]; then
  shift
  exec "$REAL_GH" "$@"
fi

# Route marker: ~/tools/fleet-routes-ensure.sh health-probes the
# fleet gh route (init prelude + 15-min keeper) and records the
# verdict here. While it says "direct", the fleet route is down, so
# go straight to the in-process signed-in gh; the ensure flips it
# back when the route recovers. A missing marker is the primary
# route. One builtin read per call — the decision itself is made
# at ensure time, never per call.
GH_ROUTE_FILE=/home/hatch/.local/state/fleet/gh-route
if [ -f "$GH_ROUTE_FILE" ]; then
  GH_ROUTE=""
  IFS= read -r GH_ROUTE <"$GH_ROUTE_FILE" || true
  if [ "$GH_ROUTE" = "direct" ]; then
    echo "gh-shim: fleet gh route is down (routes-ensure marker); using the in-process gh" >&2
    exec "$REAL_GH" "$@"
  fi
fi

FLEET_BIN=/home/hatch/tools/fleet
if [ ! -x "$FLEET_BIN" ]; then
  echo "gh-shim: fleet binary missing ($FLEET_BIN); falling back to real gh" >&2
  exec "$REAL_GH" "$@"
fi

export FLEET_GH_BIN="$REAL_GH"
export FLEET_GH_SHIM_ACTIVE=1
exec "$FLEET_BIN" gh "$@"
