#!/usr/bin/env bash
# bootstrap.sh — bring a fresh Muse sandbox back up (developing-today/code).
#
# A fresh Muse box has none of the old sandbox: no ~/tools, no ~/.config,
# no fleet binary, no laptop link. This script rebuilds that from three
# things only: the public release binaries, one Bitwarden Secrets Manager
# access token (the only secret a person hands over), and the restic
# backups of the old sandbox in R2.
#
#   curl -fsSL https://raw.githubusercontent.com/developing-today/code/main/scripts/muse/bootstrap.sh -o bootstrap.sh
#   bash bootstrap.sh            # prompts for the bws token (hidden), once
#   bash bootstrap.sh --help     # phases and flags
#
# Phases (each one is idempotent and skips what is already done):
#   tools     fleet (static musl, bridge capable), bws, restic, tailscale
#             into ~/.local/share/muse-bootstrap/bin, sha256-checked
#   token     BWS and optional 1Password service-account tokens into
#             ~/.config/bitwarden/access-token and ~/.config/op/service-account-token
#             (600), read from stdin or hidden prompts, never argv
#   restore   ~/.config from restic-sandbox-config, then every top-level
#             home entry from restic-sandbox-home (latest snapshot per
#             entry; ~/git and ~/agents only with --all)
#   tailnet   join the tailnet with tailscale.auth-key as host "muse"
#             (kernel TUN when root, else userspace with a local CONNECT
#             proxy) and point fleet's laptop link at it
#   fleet     install the new fleet at ~/tools/fleet (+ PATH links)
#   persist   survive a sandbox recycle: copy this script to
#             ~/tools/muse-bootstrap.sh and hook ~/tools/muse-boot.sh into
#             ~/tools/init.sh, so each boot restarts the tailnet and keeps
#             fleet linked (only /home/hatch survives a recycle)
#   init      run the restored ~/tools/init.sh (units, services)
#   check     fleet version/capabilities, laptop link, secrets broker,
#             bridge hello as agent "muse" (group muse)
#
# Secrets are fetched by name and never printed, logged or put in argv.
# Exit: 0 ok, 1 a phase failed (the message names it), 2 usage.
set -euo pipefail

# ---- pinned artifacts (update together; sha256 of the downloaded file) ----
FLEET_URL="${FLEET_URL:-https://github.com/developing-today/code/releases/download/fleet-static-ad0707b/fleet-x86_64-linux-musl}"
FLEET_SHA256="${FLEET_SHA256:-762bac6318329f202e47f8aeb7727cf6351e22ca93c278fd4d95377febcfa5f2}"
BWS_URL="https://github.com/bitwarden/sdk-sm/releases/download/bws-v2.1.0/bws-x86_64-unknown-linux-musl-2.1.0.zip"
BWS_SHA256="f59ee150e42b82128d437087e9bac920053c6bfddcb960d20ce9386e5ac9bba6"
RESTIC_URL="https://github.com/restic/restic/releases/download/v0.20.0/restic_0.20.0_linux_amd64.bz2"
RESTIC_SHA256="0ccd9fa7885074a24603b051e01bf9bd8b5e45d6a1791e9d9887e9e9372107f7"
TS_URL="https://pkgs.tailscale.com/stable/tailscale_1.104.7_amd64.tgz"
TS_SHA256="e56693ff74a64f2591b7e819c280635a100aae1ddfb0ef058fcd052b7cd8d16c"

R2_BASE="s3:https://234b866b99ce2b142d7963d8d139cbfa.r2.cloudflarestorage.com/supabase-backups"
SANDBOX_HOME=/home/hatch # the old sandbox's home; the restored tools hard-code it
LAPTOP_TAILNET_IP=100.99.113.103
AGENT="${FLEET_AGENT:-muse}"
AGENT_WAS_EXPLICIT=0
[ "${FLEET_AGENT+x}" = x ] && AGENT_WAS_EXPLICIT=1
TS_HOSTNAME_OVERRIDE="${TS_HOSTNAME:-}"
TS_STATE_OVERRIDE="${TS_STATE:-}"

PHASES="tools token restore tailnet fleet persist init check"
ALL=0
SELF=""
[ -f "${BASH_SOURCE[0]:-}" ] && SELF=$(readlink -f "${BASH_SOURCE[0]}")
usage() {
  sed -n '2,/^set -euo pipefail/p' "$SELF" | sed '$d' | sed 's/^# \{0,1\}//'
  echo
  echo "Flags: --only <phase[,phase]>  --skip <phase[,phase]>  --all (restore ~/git and ~/agents too)"
  exit "${1:-0}"
}
ONLY="" SKIP=""
phases_arg() { # phases_arg FLAG VALUE: a comma list of known phases, or a usage error
  case "${2-}" in "" | -*)
    echo "bootstrap: $1 needs a phase list (one of: $PHASES)" >&2
    exit 2
    ;;
  esac
  local p
  for p in ${2//,/ }; do
    case " $PHASES " in *" $p "*) ;; *)
      echo "bootstrap: $1: unknown phase '$p' (one of: $PHASES)" >&2
      exit 2
      ;;
    esac
  done
}
while [ $# -gt 0 ]; do
  case "$1" in
  --only)
    phases_arg --only "${2-}"
    ONLY=$2
    shift 2
    ;;
  --skip)
    phases_arg --skip "${2-}"
    SKIP=$2
    shift 2
    ;;
  --all)
    ALL=1
    shift
    ;;
  -h | --help) usage 0 ;;
  *)
    echo "bootstrap: unknown argument $1" >&2
    usage 2
    ;;
  esac
done
[ -z "$ONLY" ] || [ -z "$SKIP" ] || {
  echo "bootstrap: --only and --skip exclude each other" >&2
  exit 2
}
want() {
  case ",$SKIP," in *",$1,"*) return 1 ;; esac
  [ -z "$ONLY" ] && return 0
  case ",$ONLY," in *",$1,"*) return 0 ;; esac
  return 1
}
say() { printf '[bootstrap] %s\n' "$*" >&2; }
die() {
  printf '[bootstrap] FAILED: %s\n' "$*" >&2
  exit 1
}

[ "$(uname -m)" = x86_64 ] || die "this script pins x86_64 binaries; uname -m says $(uname -m)"
for c in curl tar python3 sha256sum; do command -v "$c" >/dev/null || die "needs $c on PATH"; done

HOME="${HOME:?HOME must be set}"
# Each explicitly named Fleet agent gets its own Tailscale node state and a
# stable, readable hostname. Never reuse the legacy shared "muse" identity
# for a newly named agent: that can silently move an existing node.
AGENT_SLUG=$(printf '%s' "$AGENT" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9-]/-/g; s/^-*//; s/-*$//' | cut -c1-55)
[ -n "$AGENT_SLUG" ] || die "FLEET_AGENT must contain at least one letter or number"
case "$AGENT_SLUG" in muse-*) AGENT_SLUG=${AGENT_SLUG#muse-} ;; esac
[ -n "$AGENT_SLUG" ] || die "FLEET_AGENT must contain a name after 'muse-'"
if [ -n "$TS_HOSTNAME_OVERRIDE" ]; then
  TS_HOSTNAME=$TS_HOSTNAME_OVERRIDE
elif [ "$AGENT_WAS_EXPLICIT" = 1 ]; then
  TS_HOSTNAME="muse-$AGENT_SLUG"
else
  TS_HOSTNAME=muse # preserve the legacy default for existing installations
fi
if [ -n "$TS_STATE_OVERRIDE" ]; then
  TS_STATE=$TS_STATE_OVERRIDE
elif [ "$AGENT_WAS_EXPLICIT" = 1 ]; then
  TS_STATE="$HOME/.local/state/tailscale/agents/$AGENT_SLUG"
else
  TS_STATE="$HOME/.local/state/tailscale" # preserve the legacy node state
fi
# Only $SANDBOX_HOME (a btrfs volume) survives a sandbox recycle; /, /etc,
# /root and /usr/local are wiped. Everything below is written under $HOME.
if [ "$HOME" != "$SANDBOX_HOME" ]; then
  say "warning: HOME is $HOME, not $SANDBOX_HOME; on a Muse sandbox nothing written here survives a restart (run with HOME=$SANDBOX_HOME)"
fi
# Downloaded tools live outside ~/tools so the restore (which brings back
# the old ~/tools, binaries included) never overwrites a running binary.
BIN="$HOME/.local/share/muse-bootstrap/bin"
# fleet picks its verb set from argv[0], so the download keeps the name
# "fleet" (in its own directory, off PATH) until the fleet phase installs it.
FLEET_DL="$HOME/.local/share/muse-bootstrap/fleet/fleet"
mkdir -p "$BIN" "$(dirname "$FLEET_DL")" "$HOME/tools/bin"
export PATH="$BIN:$HOME/tools/bin:$PATH"
SUDO=""
if [ "$(id -u)" != 0 ] && command -v sudo >/dev/null && sudo -n true 2>/dev/null; then SUDO="sudo -n"; fi

has_bridge() { # has_bridge BINARY -> 0 when the build carries a usable bridge verb
  "$1" capabilities 2>/dev/null | python3 -c '
import json, sys
d = json.load(sys.stdin)
sys.exit(0 if any(x["name"] == "bridge" and x["available"] for x in d.get("domains", [])) else 1)' 2>/dev/null
}
fetch() { # fetch URL SHA256 DEST
  local url=$1 sum=$2 dest=$3 tmp
  case "$sum" in @*@ | "") die "no pinned sha256 for $url (unpublished build?)" ;; esac
  tmp=$(mktemp)
  curl -fsSL --retry 4 --retry-delay 2 -o "$tmp" "$url" 2>/dev/null || {
    rm -f "$tmp"
    [ -n "${FETCH_SOFT:-}" ] && return 1
    die "download failed: $url"
  }
  echo "$sum  $tmp" | sha256sum -c --quiet - || {
    rm -f "$tmp"
    die "sha256 mismatch: $url"
  }
  mv "$tmp" "$dest"
}

# ---- tools ----
if want tools; then
  if [ ! -x "$FLEET_DL" ] || ! has_bridge "$FLEET_DL"; then
    fetch "$FLEET_URL" "$FLEET_SHA256" "$FLEET_DL"
    chmod 755 "$FLEET_DL"
  fi
  if [ ! -x "$BIN/bws" ]; then
    fetch "$BWS_URL" "$BWS_SHA256" "$BIN/bws.zip"
    python3 - "$BIN/bws.zip" "$BIN" <<'PY'
import sys, zipfile, os
z = zipfile.ZipFile(sys.argv[1])
name = next(n for n in z.namelist() if os.path.basename(n) == "bws")
open(os.path.join(sys.argv[2], "bws"), "wb").write(z.read(name))
PY
    rm -f "$BIN/bws.zip"
    chmod 755 "$BIN/bws"
  fi
  if [ ! -x "$BIN/restic" ]; then
    fetch "$RESTIC_URL" "$RESTIC_SHA256" "$BIN/restic.bz2"
    python3 -c 'import bz2,sys; open(sys.argv[2],"wb").write(bz2.open(sys.argv[1]).read())' "$BIN/restic.bz2" "$BIN/restic"
    rm -f "$BIN/restic.bz2"
    chmod 755 "$BIN/restic"
  fi
  if [ ! -x "$BIN/tailscale" ] || [ ! -x "$BIN/tailscaled" ]; then
    # Not fatal: a box that can't reach pkgs.tailscale.com still gets
    # fleet, secrets and the restore; only the tailnet phase is skipped.
    if FETCH_SOFT=1 fetch "$TS_URL" "$TS_SHA256" "$BIN/ts.tgz"; then
      tar -xzf "$BIN/ts.tgz" -C "$BIN" --strip-components=1 --wildcards '*/tailscale' '*/tailscaled'
    else
      say "tools: tailscale download failed; the tailnet phase will be skipped"
    fi
    rm -f "$BIN/ts.tgz"
  fi
  say "tools: fleet $("$FLEET_DL" version 2>/dev/null | sed -n 's/^Source rev: //p'); in $BIN: $(ls "$BIN" | tr "\n" " ")"
fi

# ---- token ----
TOKEN_FILE="$HOME/.config/bitwarden/access-token"
OP_TOKEN_FILE="$HOME/.config/op/service-account-token"
store_token() { # store_token VALUE: the supplied token, mode 600
  mkdir -p "$(dirname "$TOKEN_FILE")"
  chmod 700 "$(dirname "$TOKEN_FILE")"
  (
    umask 077
    printf '%s' "$1" >"$TOKEN_FILE.tmp"
  ) && mv -f "$TOKEN_FILE.tmp" "$TOKEN_FILE"
}
# A token passed in the environment is the one the operator supplied now:
# it replaces whatever file is on disk (a restored or stale one).
if want token && [ -n "${BWS_ACCESS_TOKEN:-}" ]; then
  store_token "$(printf '%s' "$BWS_ACCESS_TOKEN" | tr -d '[:space:]')"
fi
if want token && [ ! -s "$TOKEN_FILE" ]; then
  mkdir -p "$(dirname "$TOKEN_FILE")"
  chmod 700 "$(dirname "$TOKEN_FILE")"
  if [ -n "${BWS_ACCESS_TOKEN:-}" ]; then
    tok=$BWS_ACCESS_TOKEN
  elif [ -t 0 ]; then
    read -rsp "Bitwarden Secrets Manager access token (hidden): " tok
    echo >&2
  else tok=$(cat); fi
  tok=$(printf '%s' "$tok" | tr -d '[:space:]')
  [ -n "$tok" ] || die "token: empty"
  store_token "$tok"
  unset tok
fi
if [ -n "${OP_SERVICE_ACCOUNT_TOKEN:-}" ]; then
  mkdir -p "$(dirname "$OP_TOKEN_FILE")"
  chmod 700 "$(dirname "$OP_TOKEN_FILE")"
  (
    umask 077
    printf '%s' "$OP_SERVICE_ACCOUNT_TOKEN" >"$OP_TOKEN_FILE.tmp"
  ) &&
    mv -f "$OP_TOKEN_FILE.tmp" "$OP_TOKEN_FILE"
fi
if [ -s "$TOKEN_FILE" ]; then
  BWS_ACCESS_TOKEN=$(cat "$TOKEN_FILE")
  export BWS_ACCESS_TOKEN
fi
if [ -s "$OP_TOKEN_FILE" ]; then
  OP_SERVICE_ACCOUNT_TOKEN=$(cat "$OP_TOKEN_FILE")
  export OP_SERVICE_ACCOUNT_TOKEN
fi
secret() { # secret NAME -> value on stdout, nothing else
  [ -n "${BWS_ACCESS_TOKEN:-}" ] || die "no bws token (run the token phase)"
  bws secret list -o json 2>/dev/null | python3 -c '
import json, sys
name = sys.argv[1]
for s in json.load(sys.stdin):
    if s.get("key") == name:
        sys.stdout.write(s.get("value", "")); sys.exit(0)
sys.exit(1)' "$1" || die "secret $1 not found in Bitwarden"
}
if want token; then
  bws secret list -o json >/dev/null 2>&1 ||
    die "token: bws cannot list secrets with the token in $TOKEN_FILE (pass a fresh one: BWS_ACCESS_TOKEN=... or remove the file and rerun)"
  say "token: stored and verified"
fi

# ---- restore ----
restic_env() { # restic_env REPO PASSWORD_SECRET
  AWS_ACCESS_KEY_ID=$(secret cloudflare.r2-backups-access-key-id)
  export AWS_ACCESS_KEY_ID
  AWS_SECRET_ACCESS_KEY=$(secret cloudflare.r2-backups-secret-access-key)
  export AWS_SECRET_ACCESS_KEY
  RESTIC_PASSWORD=$(secret "$2")
  export RESTIC_PASSWORD
  export RESTIC_REPOSITORY="$R2_BASE/$1"
}
if want restore; then
  if [ "$HOME" = "$SANDBOX_HOME" ]; then ROOT=/; else
    ROOT="$HOME/restore"
    mkdir -p "$ROOT"
    say "restore: HOME is $HOME, not $SANDBOX_HOME; restoring under $ROOT (the restored tools expect $SANDBOX_HOME)"
  fi
  restic_env restic-sandbox-config restic.sandbox-config-password
  say "restore: ~/.config (latest snapshot)"
  # A box that runs as root in a restricted namespace cannot set the
  # snapshot's file ownership or some sandbox xattrs: restic reports each
  # as an error but restores the contents. Only those are tolerated.
  rlog=$(mktemp)
  if ! restic restore latest --no-lock --target "$ROOT" --path "$SANDBOX_HOME/.config" >/dev/null 2>"$rlog"; then
    other=$(grep -v -e '^[[:space:]]*$' -e 'operation not permitted' -e 'xattr' -E -e '^Fatal: There were [0-9]+ errors' "$rlog" | head -5 || true)
    if [ -n "$other" ] || ! grep -q 'operation not permitted' "$rlog"; then
      { [ -n "$other" ] && printf "%s\n" "$other" || tail -5 "$rlog"; } >&2
      rm -f "$rlog"
      die "restore: ~/.config"
    fi
    say "restore: ~/.config restored; $(grep -cE 'operation not permitted|xattr' "$rlog") ownership/xattr errors ignored (this box cannot set them)"
  fi
  rm -f "$rlog"
  # The snapshot can carry old credentials; keep the values supplied for this run.
  if [ "$ROOT" = / ]; then
    store_token "$BWS_ACCESS_TOKEN"
    if [ -n "${OP_SERVICE_ACCOUNT_TOKEN:-}" ]; then
      mkdir -p "$(dirname "$OP_TOKEN_FILE")"
      chmod 700 "$(dirname "$OP_TOKEN_FILE")"
      (
        umask 077
        printf '%s' "$OP_SERVICE_ACCOUNT_TOKEN" >"$OP_TOKEN_FILE.tmp"
      ) &&
        mv -f "$OP_TOKEN_FILE.tmp" "$OP_TOKEN_FILE"
    fi
  fi
  restic_env restic-sandbox-home restic.sandbox-home-password
  restic snapshots --no-lock --json >"$HOME/.bootstrap-snapshots.json" || die "restore: cannot list home snapshots"
  python3 - "$HOME/.bootstrap-snapshots.json" "$SANDBOX_HOME" "$ALL" >"$HOME/.bootstrap-restore-plan" <<'PY'
import json, sys
snaps, home, everything = json.load(open(sys.argv[1])), sys.argv[2], sys.argv[3] == "1"
latest = {}
for s in snaps:
    if len(s["paths"]) != 1:
        continue
    p = s["paths"][0]
    if p not in latest or s["time"] > latest[p]["time"]:
        latest[p] = s
for p in sorted(latest):
    rel = p[len(home) + 1:] if p.startswith(home + "/") else None
    if rel is None or rel == ".config":
        continue
    top = rel.split("/")[0]
    if not everything and top in ("git", "agents"):
        continue
    print(latest[p]["short_id"], p)
PY
  n=$(wc -l <"$HOME/.bootstrap-restore-plan")
  i=0
  say "restore: $n home entries (latest snapshot each)"
  while read -r sid path; do
    i=$((i + 1))
    restic restore "$sid" --no-lock --target "$ROOT" >/dev/null 2>&1 || say "restore: entry $i/$n $path FAILED (continuing)"
  done <"$HOME/.bootstrap-restore-plan"
  rm -f "$HOME/.bootstrap-snapshots.json"
  unset RESTIC_PASSWORD AWS_SECRET_ACCESS_KEY AWS_ACCESS_KEY_ID
  say "restore: done (plan kept at ~/.bootstrap-restore-plan)"
fi

# ---- keep the OP credential available to later Muse shell sessions ----
# The script and tokens live under persistent HOME on Muse. This hook only
# reads the 0600 token file; it never prints or exports it to logs.
OP_ENV="$HOME/tools/op-env.sh"
cat >"$OP_ENV.tmp" <<'EOF'
#!/usr/bin/env bash
if [ -z "${OP_SERVICE_ACCOUNT_TOKEN:-}" ] && [ -r "$HOME/.config/op/service-account-token" ]; then
  IFS= read -r OP_SERVICE_ACCOUNT_TOKEN <"$HOME/.config/op/service-account-token" || :
  export OP_SERVICE_ACCOUNT_TOKEN
fi
EOF
chmod 700 "$OP_ENV.tmp" && mv -f "$OP_ENV.tmp" "$OP_ENV"
SHELL_ENV="$HOME/tools/shell-env.sh"
if [ -f "$SHELL_ENV" ] && ! grep -q 'muse-op-token' "$SHELL_ENV"; then
  cat >>"$SHELL_ENV" <<'EOF'

# muse-op-token: load the service account for headless Fleet secrets resolution.
[ -r "$HOME/tools/op-env.sh" ] && . "$HOME/tools/op-env.sh"
EOF
fi

# ---- tailnet ----
TS_SOCK="$TS_STATE/tailscaled.sock"
if want tailnet && { [ ! -x "$BIN/tailscaled" ] || [ ! -x "$BIN/tailscale" ]; }; then
  say "tailnet: no tailscale binaries (download failed); skipping, the laptop link will not work"
elif want tailnet; then
  mkdir -p "$TS_STATE"
  if ! tailscale --socket="$TS_SOCK" status >/dev/null 2>&1; then
    MODE=userspace
    if [ -c /dev/net/tun ] && { [ "$(id -u)" = 0 ] || [ -n "$SUDO" ]; }; then MODE=kernel; fi
    # A mode a previous run recorded has already proven itself on
    # this host; let it override the probe. In the muse sandbox the
    # probe picks kernel (a TUN device exists and we run as root)
    # but creating the interface is forbidden, so every cold start
    # burned ~11s on a doomed kernel daemon before the userspace
    # fallback — time the 2026-11-22 05:25 keeper restart did not
    # have under load.
    RECORDED_MODE=$(cat "$TS_STATE/mode" 2>/dev/null || echo "")
    [ "$RECORDED_MODE" = userspace ] && MODE=userspace
    if pgrep -f -- "--state=$TS_STATE/tailscaled.state" >/dev/null 2>&1; then
      MODE=$(cat "$TS_STATE/mode" 2>/dev/null || echo "$MODE") # daemon alive, only logged out
    elif [ "$MODE" = kernel ]; then
      rm -f "$TS_SOCK" # a recycle leaves the old socket file on the durable volume
      # Rotate, never truncate: the previous daemon's log is the
      # only evidence when a start fails (the 2026-11-22 05:25
      # attempt's log was destroyed by the next start's `>`).
      [ -f "$TS_STATE/tailscaled.log" ] && mv "$TS_STATE/tailscaled.log" "$TS_STATE/tailscaled.log.prev"
      $SUDO setsid nohup "$BIN/tailscaled" --state="$TS_STATE/tailscaled.state" --socket="$TS_SOCK" \
        >"$TS_STATE/tailscaled.log" 2>&1 </dev/null &
    else
      rm -f "$TS_SOCK"
      [ -f "$TS_STATE/tailscaled.log" ] && mv "$TS_STATE/tailscaled.log" "$TS_STATE/tailscaled.log.prev"
      setsid nohup "$BIN/tailscaled" --tun=userspace-networking --state="$TS_STATE/tailscaled.state" \
        --socket="$TS_SOCK" --outbound-http-proxy-listen=127.0.0.1:3130 \
        >"$TS_STATE/tailscaled.log" 2>&1 </dev/null &
    fi
    echo "$MODE" >"$TS_STATE/mode"
    # The node key in $TS_STATE survives a recycle, so after a restart
    # tailscaled logs itself back in; the auth key is only for a new node.
    ts_backend() { "$BIN/tailscale" --socket="$TS_SOCK" status --json 2>/dev/null |
      python3 -c 'import json,sys; print(json.load(sys.stdin).get("BackendState",""))' 2>/dev/null || true; }
    state=""
    # 60 iterations, not 30: each poll spawns tailscale + python3,
    # and under sandbox CPU contention (concurrent lane builds) a
    # single poll can take several seconds — 30 polls elapsed
    # before a healthy daemon reached Running in the 2026-11-22
    # 05:25 restart, and the phase died on the auth-key path.
    for _ in $(seq 1 60); do
      state=$(ts_backend)
      case "$state" in Running | NeedsLogin | NoState | Stopped) [ "$state" = NoState ] || break ;; esac
      sleep 1
    done
    if [ "$state" != Running ]; then
      keyfile=$(mktemp)
      (
        umask 077
        secret tailscale.auth-key >"$keyfile"
      )
      TSUDO=""
      [ "$MODE" = kernel ] && TSUDO=$SUDO
      $TSUDO "$BIN/tailscale" --socket="$TS_SOCK" up --auth-key="file:$keyfile" --hostname="$TS_HOSTNAME" \
        --operator="$(id -un)" --accept-dns=false >/dev/null 2>&1 || {
        rm -f "$keyfile"
        die "tailnet: tailscale up refused (see $TS_STATE/tailscaled.log)"
      }
      rm -f "$keyfile"
    fi
  fi
  MODE=$(cat "$TS_STATE/mode" 2>/dev/null || echo kernel)
  # fleet's laptop link: kernel mode dials the laptop directly; userspace
  # mode goes through tailscaled's CONNECT proxy, which fleet expects on
  # the tailnet proxy port 3130 of HTTPS_PROXY's host.
  FT="$HOME/.config/fleet/fleet.toml"
  mkdir -p "$(dirname "$FT")"
  touch "$FT"
  python3 - "$FT" "$MODE" <<'PY'
import re, sys
path, mode = sys.argv[1], sys.argv[2]
text = open(path).read()
want = "true" if mode == "kernel" else "false"
if re.search(r"(?m)^\[proxy\]\s*$", text):
    sec = re.search(r"(?ms)^\[proxy\]\s*$(.*?)(?=^\[|\Z)", text)
    body = sec.group(1)
    if re.search(r"(?m)^direct\s*=", body):
        body2 = re.sub(r"(?m)^direct\s*=.*$", f"direct = {want}", body)
    else:
        body2 = f"\ndirect = {want}" + body
    text = text[:sec.start(1)] + body2 + text[sec.end(1):]
else:
    text = text.rstrip("\n") + ("\n\n" if text.strip() else "") + f"[proxy]\ndirect = {want}\n"
open(path, "w").write(text)
PY
  if [ "$MODE" = userspace ]; then
    echo 'export HTTPS_PROXY=http://127.0.0.1:3130' >"$TS_STATE/fleet-proxy.env"
    say "tailnet: userspace mode; start fleet proxy with HTTPS_PROXY from $TS_STATE/fleet-proxy.env"
  else
    # Fleet's tailnet tunnels prefer this file over the ambient proxy,
    # so a kernel-mode seat must not keep one from an earlier run.
    rm -f "$TS_STATE/fleet-proxy.env"
  fi
  EFFECTIVE_HOSTNAME=$("$BIN/tailscale" --socket="$TS_SOCK" status --json 2>/dev/null |
    python3 -c 'import json,sys; print(json.load(sys.stdin).get("Self",{}).get("HostName",""))' 2>/dev/null || true)
  [ -n "$EFFECTIVE_HOSTNAME" ] || EFFECTIVE_HOSTNAME=$TS_HOSTNAME
  if [ "$MODE" = userspace ]; then
    say "tailnet: up as $EFFECTIVE_HOSTNAME (userspace mode; HTTP CONNECT works; direct Tailscale UDP is unavailable (Iroh public UDP may still work))"
  else
    say "tailnet: up as $EFFECTIVE_HOSTNAME (kernel mode; direct UDP paths are available when NAT/firewall permit)"
  fi
  say "tailnet: node state is isolated at $TS_STATE; a hostname collision may get a Tailscale suffix and will not rename another node"
  if PING_RESULT=$("$BIN/tailscale" --socket="$TS_SOCK" ping --c 1 "$LAPTOP_TAILNET_IP" 2>&1); then
    case "$PING_RESULT" in
    *"via DERP"*) say "tailnet: laptop is reachable through DERP; direct Tailscale UDP is not established" ;;
    *"via "* | *"direct"*) say "tailnet: laptop probe succeeded over a direct Tailscale path" ;;
    *) say "tailnet: laptop probe succeeded; route type was not reported" ;;
    esac
  else
    say "tailnet: laptop probe to $LAPTOP_TAILNET_IP did not succeed yet; Cloudflare HTTPS remains the fallback"
  fi
fi

# ---- fleet ----
if want fleet; then
  [ -x "$FLEET_DL" ] || die "fleet: no downloaded binary (run the tools phase)"
  if [ -e "$HOME/tools/fleet" ] && [ ! -w "$HOME/tools/fleet" ]; then
    [ -n "$SUDO" ] || [ "$(id -u)" = 0 ] || die "fleet: $HOME/tools/fleet is not writable and there is no sudo"
    $SUDO mv -f "$HOME/tools/fleet" "$HOME/tools/fleet.old"
  fi
  if ! cmp -s "$FLEET_DL" "$HOME/tools/fleet"; then
    install -m 755 "$FLEET_DL" "$HOME/tools/fleet.tmp" && mv -f "$HOME/tools/fleet.tmp" "$HOME/tools/fleet"
  fi
  for l in fleet repo-queue; do ln -sfn "$HOME/tools/fleet" "$HOME/tools/bin/$l"; done
  hash -r
  say "fleet: $("$HOME/tools/fleet" version | sed -n 's/^Source rev: //p') at ~/tools/fleet"
fi

# ---- persist ----
# A recycle keeps $SANDBOX_HOME (tools, state, token) but kills every
# process and wipes /etc. ~/tools/init.sh already runs on every boot (cron
# sandbox-boot-init, every 5 min), so hook a small boot script into it that
# reruns the idempotent tailnet and fleet phases from a saved copy of this
# script. When the tailnet is already up the hook returns in about a second.
if want persist; then
  if [ -n "$SELF" ]; then
    [ "$SELF" = "$HOME/tools/muse-bootstrap.sh" ] ||
      install -m 755 "$SELF" "$HOME/tools/muse-bootstrap.sh"
  elif [ ! -f "$HOME/tools/muse-bootstrap.sh" ]; then
    die "persist: run this script from a file (bash bootstrap.sh), not a pipe, so it can keep a copy"
  fi
  mkdir -p "$HOME/logs"
  cat >"$HOME/tools/muse-boot.sh.tmp" <<EOF
#!/usr/bin/env bash
# muse-boot.sh — written by scripts/muse/bootstrap.sh (persist phase).
# Run from ~/tools/init.sh on every boot: restarts the tailnet after a
# sandbox recycle and keeps the bridge-capable fleet linked. Idempotent.
export HOME=$HOME
exec >>"\$HOME/logs/muse-boot.log" 2>&1 </dev/null
echo "== \$(date -u +%FT%TZ)"
# One run at a time: init.sh fires every few minutes.
if command -v flock >/dev/null; then
  exec flock -n "\$HOME/tools/.muse-boot.lock" timeout 300 bash "\$HOME/tools/muse-bootstrap.sh" --only tailnet,fleet
fi
exec timeout 300 bash "\$HOME/tools/muse-bootstrap.sh" --only tailnet,fleet
EOF
  chmod 755 "$HOME/tools/muse-boot.sh.tmp" && mv -f "$HOME/tools/muse-boot.sh.tmp" "$HOME/tools/muse-boot.sh"
  INIT="$HOME/tools/init.sh"
  if [ ! -f "$INIT" ]; then
    printf '#!/usr/bin/env bash\n' >"$INIT"
    chmod 755 "$INIT"
  fi
  if ! grep -q 'muse-boot.sh' "$INIT"; then
    # Right after the shebang, so an early exit in init.sh cannot skip it.
    python3 - "$INIT" "$HOME/tools/muse-boot.sh" <<'PY'
import sys
p, boot = sys.argv[1], sys.argv[2]
lines = open(p).read().splitlines(keepends=True)
hook = ('# muse-bootstrap: bring the tailnet back after a recycle (scripts/muse/bootstrap.sh)\n'
        f'[ -x {boot} ] && {{ {boot} || true; }}\n')
at = 1 if lines and lines[0].startswith("#!") else 0
lines.insert(at, hook)
open(p + ".tmp", "w").write("".join(lines))
PY
    chmod --reference="$INIT" "$INIT.tmp" && mv -f "$INIT.tmp" "$INIT"
    say "persist: hooked ~/tools/muse-boot.sh into ~/tools/init.sh"
  fi
  say "persist: a restart brings back the tailnet and fleet from ~/tools/muse-bootstrap.sh (log ~/logs/muse-boot.log)"
fi

# ---- init ----
if want init; then
  if [ -x "$HOME/tools/init.sh" ]; then
    say "init: running ~/tools/init.sh"
    if [ -n "$SUDO" ]; then
      $SUDO env HOME="$HOME" bash "$HOME/tools/init.sh" || say "init: init.sh returned non-zero (see ~/logs/init.log)"
    else bash "$HOME/tools/init.sh" || say "init: init.sh returned non-zero (see ~/logs/init.log)"; fi
  else
    say "init: no ~/tools/init.sh (restore skipped or failed); nothing to run"
  fi
fi

# ---- check ----
if want check; then
  rc=0
  say "check: running"
  has_bridge "$(command -v fleet)" || {
    say "check: fleet has no bridge capability"
    rc=1
  }
  # Reach the laptop agent port the way fleet does: directly in kernel
  # mode, or by CONNECT through tailscaled's local proxy in userspace
  # mode. Only a TCP connect to the laptop (or the proxy's 200 to the
  # CONNECT) counts; a proxy's refusal does not.
  TS_PROXY=""
  if [ "$(cat "$TS_STATE/mode" 2>/dev/null)" = userspace ]; then TS_PROXY=127.0.0.1:3130; fi
  if python3 -c '
import socket, sys
host, port, proxy = sys.argv[1], int(sys.argv[2]), sys.argv[3]
try:
    if proxy:
        ph, pp = proxy.rsplit(":", 1)
        s = socket.create_connection((ph, int(pp)), timeout=10)
        s.sendall(("CONNECT %s:%d HTTP/1.1\r\nHost: %s:%d\r\n\r\n" % (host, port, host, port)).encode())
        line = s.recv(256).split(b"\r\n", 1)[0].split()
        sys.exit(0 if len(line) > 1 and line[1] == b"200" else 1)
    socket.create_connection((host, port), timeout=10).close()
except OSError:
    sys.exit(1)
' "$LAPTOP_TAILNET_IP" 4799 "$TS_PROXY"; then
    say "check: laptop agent port answers on the tailnet"
  else
    say "check: laptop agent port did not answer"
    rc=1
  fi
  if fleet secrets get tailscale.auth-key >/dev/null 2>&1; then say "check: fleet secrets resolve"; else
    say "check: fleet secrets get failed"
    rc=1
  fi
  if fleet secrets list >/dev/null 2>&1; then say "check: session broker answers"; else say "check: no session broker answering yet (init.sh starts it; bws fallback still works)"; fi
  if command -v op >/dev/null 2>&1 && [ -n "${OP_SERVICE_ACCOUNT_TOKEN:-}" ] &&
    OP_SERVICE_ACCOUNT_TOKEN="$OP_SERVICE_ACCOUNT_TOKEN" op whoami >/dev/null 2>&1; then
    say "check: 1Password CLI and service account answer"
  else
    say "check: 1Password CLI/token did not verify (op is restored from ~/tools or login data is missing)"
    rc=1
  fi
  if fleet bridge hello --agent "$AGENT" --groups muse >/dev/null 2>&1; then
    say "check: bridge hello as $AGENT ok; unread commands: $(fleet bridge inbox --agent "$AGENT" --commands 2>/dev/null | python3 -c 'import json,sys;d=json.load(sys.stdin);print(len(d if isinstance(d,list) else d.get("messages",d.get("items",[]))))' 2>/dev/null || echo '?')"
  else
    say "check: bridge hello failed"
    rc=1
  fi
  [ "$rc" = 0 ] || die "check: see the lines above"
  say "check: all good"
fi
