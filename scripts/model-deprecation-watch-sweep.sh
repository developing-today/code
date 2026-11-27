#!/usr/bin/env bash
# model-deprecation-watch weekly sweep (developing-today/code#14236, #7013).
# Re-fetches the provider lifecycle surfaces registered in
# doc/model-deprecation-watch.md, snapshots them, and appends a dated
# summary line. A human/lane reviews the log and updates the registry doc.
set -u
OUT_DIR="${MODEL_DEPRECATION_WATCH_DIR:-$HOME/.local/share/model-deprecation-watch}"
mkdir -p "$OUT_DIR/snapshots"
STAMP="$(date +%Y%m%dT%H%M%S)"
fetch() {
  local name="$1" url="$2"
  curl -fsSL --max-time 60 -A "Mozilla/5.0 (fleet-watch)" "$url" \
    -o "$OUT_DIR/snapshots/${STAMP}-${name}.html" 2>>"$OUT_DIR/sweep-errors.log" &&
    echo "ok $name" || echo "FAIL $name"
}
{
  echo "=== sweep $STAMP ==="
  fetch anthropic-deprecations "https://platform.claude.com/docs/en/about-claude/model-deprecations"
  fetch openai-deprecations "https://platform.openai.com/docs/deprecations"
  fetch google-models "https://ai.google.dev/gemini-api/docs/models"
  fetch deepseek-pricing "https://api-docs.deepseek.com/quick_start/pricing"
  fetch mistral-models "https://docs.mistral.ai/getting-started/models/models_overview"
  echo "snapshots in $OUT_DIR/snapshots/${STAMP}-*"
} >>"$OUT_DIR/sweep.log" 2>&1
