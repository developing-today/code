# laptop-scratch-sweep — daily age-based reaping of test/build
# scratch that nothing else ages out (darwin-nix#373).
#
# Two trees grow without bound on the laptop because their writers
# create scratch and never reap it:
#   1. ~/.local/share/opencode/tmp — opencode test/build scratch:
#      ~7,500 new entries a day (core-test dirs ~2.4 GB each, cargo
#      target dirs, lane scratch). The 2026-11-11 disk catch deleted
#      15,493 entries older than 72h (~80 GB) by hand; this sweep is
#      that same rule, standing.
#   2. /tmp/mcpx-bin-* — per-run mcpx binary dirs; the catch applied
#      a 24h rule there.
#
# Rules (top-level entries only, by mtime):
#   - entries under ~/.local/share/opencode/tmp older than 72h
#   - /tmp/mcpx-bin-* directories older than 24h
#
# Safety: the managed roots are fixed paths; only entries strictly
# older than the threshold are removed (an entry still being
# written has a fresh mtime); the roots themselves are never
# removed; ~/.local/state, the opencode session store (which lives
# directly under ~/.local/share/opencode, NOT in tmp/), and
# checkout working files are never touched — the find is depth-1
# inside the tmp root, and in /tmp matches only the mcpx-bin-
# prefix. No set -e: one stubborn entry must not stop the sweep;
# its rm error stays visible in the journal.
#
# Test seams: OPENCODE_TMP_DIR / MCPX_TMP_DIR override the roots,
# OPENCODE_TMP_MAX_AGE_HOURS / MCPX_MAX_AGE_HOURS the thresholds.
set -uo pipefail

OPENCODE_TMP="${OPENCODE_TMP_DIR:-$HOME/.local/share/opencode/tmp}"
OPENCODE_MAX_AGE_HOURS="${OPENCODE_TMP_MAX_AGE_HOURS:-72}"
MCPX_TMP="${MCPX_TMP_DIR:-/tmp}"
MCPX_MAX_AGE_HOURS="${MCPX_MAX_AGE_HOURS:-24}"

sweep_opencode=0
if [ -d "$OPENCODE_TMP" ]; then
  while IFS= read -r -d '' entry; do
    if rm -rf -- "$entry"; then
      sweep_opencode=$((sweep_opencode + 1))
    fi
  done < <(find "$OPENCODE_TMP" -mindepth 1 -maxdepth 1 \
    -mmin "+$((OPENCODE_MAX_AGE_HOURS * 60))" -print0)
fi

sweep_mcpx=0
if [ -d "$MCPX_TMP" ]; then
  while IFS= read -r -d '' entry; do
    if rm -rf -- "$entry"; then
      sweep_mcpx=$((sweep_mcpx + 1))
    fi
  done < <(find "$MCPX_TMP" -mindepth 1 -maxdepth 1 -type d \
    -name 'mcpx-bin-*' -mmin "+$((MCPX_MAX_AGE_HOURS * 60))" -print0)
fi

echo "laptop-scratch-sweep: opencode-tmp removed=$sweep_opencode (>${OPENCODE_MAX_AGE_HOURS}h) mcpx-bin removed=$sweep_mcpx (>${MCPX_MAX_AGE_HOURS}h)"
