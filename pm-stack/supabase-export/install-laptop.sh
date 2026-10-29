#!/usr/bin/env bash
# install-laptop.sh — install the Supabase usage exporter on the laptop
# (developing-today/code#1050). Copies the producer beside its output
# directory and enables the 15-minute timer beside the #1151 gatherer tick.
# The management token is resolved BY NAME from Bitwarden at runtime
# (SUPABASE_TOKEN_SECRET_NAME, default supabase.access-token-muse-cli);
# this script never reads, prints, or stores its value.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
target="${SUPABASE_EXPORT_DIR:-$HOME/atlas/supabase-export}"
units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

mkdir -p "$target" "$units"
install -m 0644 "$here/supabase-quota-export.mjs" "$target/supabase-quota-export.mjs"
# Render the unit's paths from the selected install directory — copying it
# unchanged would leave the timer pointing at the default directory.
bash_bin="$(command -v bash || true)"
if [[ -z $bash_bin ]]; then
  echo "install-laptop: bash not found on PATH" >&2
  exit 1
fi
bun_bin="$(command -v bun || true)"
if [[ -z $bun_bin && -x $HOME/.bun/bin/bun ]]; then bun_bin="$HOME/.bun/bin/bun"; fi
if [[ -z $bun_bin ]]; then
  echo "install-laptop: bun not found on PATH or at ~/.bun/bin/bun" >&2
  exit 1
fi
sed -e "s|@EXPORT_DIR@|$target|g" -e "s|@BUN@|$bun_bin|g" -e "s|@BASH@|$bash_bin|g" "$here/supabase-quota-export.service" > "$units/supabase-quota-export.service"
chmod 0644 "$units/supabase-quota-export.service"
if grep -q '@EXPORT_DIR@\|@BUN@\|@BASH@' "$units/supabase-quota-export.service"; then
  echo "install-laptop: service unit still contains an unrendered placeholder after rendering" >&2
  exit 1
fi
install -m 0644 "$here/supabase-quota-export.timer" "$units/supabase-quota-export.timer"

systemctl --user daemon-reload
systemctl --user enable --now supabase-quota-export.timer
systemctl --user list-timers supabase-quota-export.timer --no-pager
echo "supabase-export: installed to $target; first tick on the next quarter hour"
