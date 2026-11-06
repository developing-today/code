#!/usr/bin/env bash
# Guard the local-proxy ports against silent drift (#434, #435).
#
# Each proxy's port is defined once, in the let block of
# home/common/default.nix, and flows to the systemd service environment
# and the shell session variables. Two places keep a literal copy BY
# DESIGN, as the documented fallback for launches that never see those
# variables:
#
#   - bin/<name>-proxy.mjs: parseProxyPort('<VAR>', <port>) (or the older
#     process.env.<VAR> || '<port>' form)
#   - .opencode/opencode.jsonc: {env:<VAR>:-<port>} in the provider
#     baseURL (opencode's {env:VAR} substitution has no fallback of its
#     own; the :- form is what keeps a non-shell launch — systemd
#     service, desktop, VS Code — from resolving to an empty port).
#
# This script fails, naming the offender, when any copy disagrees with
# the let-block definition, when a baseURL loses its :- fallback, or
# when a bare {env:<VAR>} or a literal proxy port reappears in
# opencode.jsonc. The route proxy has no baseURL in this repo.
#
# Retired 2026-10-30 (bce524fc): agy-proxy (:18743) and
# antigravity-ls-proxy (:18745) — their lets and units were removed and
# antigravity-direct (:18751, below) is the single Antigravity door.
# Retired 2026-11-04 (opencode#1414): the antigravity-direct door itself
# is gone — let, unit, session var, bin program and this row removed in
# one change; the provider id lives on over the in-fork native provider.
# Their bin/ scripts remained in the repo as history until the
# 2026-11-06 proxy-infra decommission removed them; their disabled
# opencode.jsonc providers keep the old fallbacks; see
# the retirement note in home/common/default.nix. They are deliberately
# NOT rows here — restoring one means restoring its let from git
# history and re-adding its row in the same change (#2961).
#
# Retired 2026-11-04 (opencode#1410): the clef door (:18742) is gone
# the same way — let, unit + socket, session var, bin program and
# this row removed in one change; the OpenChamber classifier rides
# the in-fork facade's /v1/systemone on the opencode server
# (127.0.0.1:13420), proven byte-identical to the door on the
# recorded classifier probe before the repoint.
#
# Retired 2026-11-04 (opencode#1153): the route proxy (:18750) is
# gone the same way — let, unit, bin program and this row removed
# in one change; its only live consumer (OpenClaw) rides the
# server facade at 127.0.0.1:13420/v1.
#
# Retired 2026-11-03 (#5240): the design-code door (:18746) was removed
# outright (7b51087cc removed its let and unit; a0755459e removed its
# opencode.jsonc door shell), so its row is gone from the table below,
# same pattern as the 2026-10-30 retirements; bin/design-code-proxy.mjs
# remained as history until the 2026-11-06 proxy-infra decommission
# removed it. The codex door program (:18744) is NOT retired —
# only its opencode.jsonc provider was (1dad07b4a re-seated the
# openai-codex routes onto openai-plan) — so the CODEX row keeps its
# let and bin/ fallback checks with has_config "no": there is no
# baseURL left in opencode.jsonc for it to match.
#
# Retired 2026-11-06 (Drew, proxy-infra decommission): the
# commandcode door (:18748) and the claude-code door (:18749) are
# gone the same way — let, unit, session var, bin program and their
# rows below removed in one change. Both providers were repointed
# to the fork's native wire-emulation providers on 2026-11-05
# (code#6117, opencode#1749); this guard had been failing on their
# stale rows since that repoint removed the jsonc baseURLs.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
nix_file="$repo_root/home/common/default.nix"
config_file="$repo_root/.opencode/opencode.jsonc"

# var | let binding | proxy script | has an opencode.jsonc baseURL
proxies=(
  "CODEX_PROXY_PORT|codexProxyPort|bin/codex-proxy.mjs|no"
  "MUSE_PROXY_PORT|museProxyPort|bin/muse-proxy.mjs|yes"
)

failed=0
for entry in "${proxies[@]}"; do
  IFS='|' read -r var let_name script has_config <<<"$entry"

  let_port="$(grep -E "^[[:space:]]*${let_name} = [0-9]+;" "$nix_file" | grep -oE '[0-9]+' | head -1 || true)"
  if [[ -z $let_port ]]; then
    echo "::error::${let_name} definition not found in home/common/default.nix."
    failed=1
    continue
  fi

  if ! grep -qF "parseProxyPort('${var}', ${let_port})" "$repo_root/$script" &&
    ! grep -qF "parseProxyPort(\"${var}\", ${let_port})" "$repo_root/$script" &&
    ! grep -qF "process.env.${var} || '${let_port}'" "$repo_root/$script" &&
    ! grep -qF "process.env.${var} || ${let_port}" "$repo_root/$script"; then
    echo "::error::$script fallback for ${var} does not match the ${let_name} definition (${let_port}) in home/common/default.nix."
    failed=1
  fi

  if [[ $has_config == yes ]] && ! grep -qF "{env:${var}:-${let_port}}" "$config_file"; then
    echo "::error::.opencode/opencode.jsonc has no {env:${var}:-${let_port}} baseURL — the fallback must match the ${let_name} definition (${let_port}); a bare {env:${var}} resolves to an empty port outside a home-manager shell (#434)."
    failed=1
  fi
  if grep -qF "{env:${var}}" "$config_file"; then
    echo "::error::.opencode/opencode.jsonc uses a bare {env:${var}} with no :- fallback (#434)."
    failed=1
  fi
done

if grep -qE '127\.0\.0\.1:187(4[0-9]|5[01])' "$config_file"; then
  echo "::error::.opencode/opencode.jsonc hardcodes a literal proxy port — use {env:<VAR>:-<port>} so the copies cannot drift (#435)."
  failed=1
fi

if [[ $failed -ne 0 ]]; then
  exit 1
fi

echo "proxy port fallbacks match the home/common/default.nix definitions (${#proxies[@]} proxies)."
