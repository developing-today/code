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
# opencode.jsonc. Clef has no opencode.jsonc consumer (its models ride
# the Cloudflare gateway), so only its script fallback is checked.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
nix_file="$repo_root/home/common/default.nix"
config_file="$repo_root/.opencode/opencode.jsonc"

# var | let binding | proxy script | has an opencode.jsonc baseURL
proxies=(
  "CLEF_PROXY_PORT|clefProxyPort|bin/clef-proxy.mjs|no"
  "AGY_PROXY_PORT|agyProxyPort|bin/agy-proxy.mjs|yes"
  "CODEX_PROXY_PORT|codexProxyPort|bin/codex-proxy.mjs|yes"
  "ANTIGRAVITY_LS_PROXY_PORT|antigravityLsProxyPort|bin/antigravity-ls-proxy.mjs|yes"
  "DESIGN_CODE_PROXY_PORT|designCodeProxyPort|bin/design-code-proxy.mjs|yes"
  "MUSE_PROXY_PORT|museProxyPort|bin/muse-proxy.mjs|yes"
  "COMMANDCODE_PROXY_PORT|commandcodeProxyPort|bin/commandcode-proxy.mjs|yes"
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
    ! grep -qF "process.env.${var} || '${let_port}'" "$repo_root/$script"; then
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

if grep -qE '127\.0\.0\.1:1874[0-9]' "$config_file"; then
  echo "::error::.opencode/opencode.jsonc hardcodes a literal proxy port — use {env:<VAR>:-<port>} so the copies cannot drift (#435)."
  failed=1
fi

if [[ $failed -ne 0 ]]; then
  exit 1
fi

echo "proxy port fallbacks match the home/common/default.nix definitions (${#proxies[@]} proxies)."
