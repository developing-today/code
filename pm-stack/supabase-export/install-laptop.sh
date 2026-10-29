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
install -m 0644 "$here/supabase-quota-export.service" "$units/supabase-quota-export.service"
install -m 0644 "$here/supabase-quota-export.timer" "$units/supabase-quota-export.timer"

systemctl --user daemon-reload
systemctl --user enable --now supabase-quota-export.timer
systemctl --user list-timers supabase-quota-export.timer --no-pager
echo "supabase-export: installed to $target; first tick on the next quarter hour"
