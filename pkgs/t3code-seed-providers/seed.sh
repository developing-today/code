#!/usr/bin/env bash
# Seed t3code's provider instances declaratively.
#
# t3 keeps its settings in $T3CODE_HOME/userdata/settings.json and WRITES to it
# at runtime, so it cannot be a read-only home.file symlink into /nix/store --
# that would stop t3 saving any setting. Instead this merges our provider
# instances into whatever is already there, before t3 starts.
#
# Hand-editing settings.json is supported upstream (verified: a hand-written
# file with five providerInstances survived a restart with no parse warnings,
# and upstream's own code mentions "Bitbucket tokens hand-edited into
# settings.json").
#
# Schema, from the binary:
#   providerInstances: Record<ProviderInstanceId, ProviderInstanceConfig>
#   ProviderInstanceConfig = {
#     driver, displayName?, accentColor?, environment?, enabled?, config?
#   }
# The default instance id for a driver is the driver kind itself, so the
# "opencode" id below is the back-compat single-instance default and
# "opencode-v2" is an additional instance.
#
# Existing entries are left alone -- this only adds instances that are absent,
# so anything configured in the UI survives.

set -euo pipefail

STATE="${T3CODE_HOME:-$HOME/.local/share/t3code}/userdata"
SETTINGS="$STATE/settings.json"
mkdir -p "$STATE"
[ -e "$SETTINGS" ] || echo '{}' >"$SETTINGS"

python3 - "$SETTINGS" <<'PY'
import json, os, pathlib, sys

path = pathlib.Path(sys.argv[1])
try:
    doc = json.loads(path.read_text() or "{}")
except Exception:
    sys.exit(0)  # don't clobber something we can't parse

def env(name):
    v = os.environ.get(name, "").strip()
    return v or None

# Default model per instance.
#
# t3's built-in DEFAULT_MODEL_BY_PROVIDER is:
#   codex -> gpt-6-astra, claudeAgent -> claude-fable-5-1, cursor -> auto,
#   grok -> grok-build, opencode -> openai/gpt-5, antigravity -> antigravity-default
#
# gpt-5 and gpt-6-astra are both superseded; Luna is the current long-context
# line (1,050,000 context / 128,000 output on every provider except Databricks).
# Set explicitly so new instances do not land on a stale default.
#
# claudeAgent is deliberately NOT given a Luna model: that driver runs Claude
# Code, which can only serve Anthropic models. Pointing it at an OpenAI slug
# would just break it. Its "fable" default is left alone.
OPENCODE_MODEL = "openai/gpt-6-luna"   # opencode driver uses provider/model form
CODEX_MODEL = "gpt-6-luna"             # codex driver uses a bare slug

desired = {
    # No bare "opencode" instance. nixos/environment puts this same build on
    # PATH as `opencode`, so a PATH-resolved instance would be byte-identical to
    # the explicit opencode-v2 below -- two entries, one binary, one of them
    # mislabelled. The build is pinned by binaryPath further down.
    "codex": {
        "driver": "codex",
        "displayName": "Codex",
        "enabled": True,
        "config": {"model": CODEX_MODEL},
    },
    "claudeAgent": {"driver": "claudeAgent", "displayName": "Claude Code", "enabled": True},
    "antigravity": {"driver": "antigravity", "displayName": "Antigravity", "enabled": True},
}

# Grok is only enabled when credentials are actually present. t3's grok driver
# reads ~/.grok/auth.json (keyed on https://auth.x.ai::...) or $GROK_AUTH /
# $GROK_API_KEY; with none of those an enabled instance just shows up as a
# permanently unhealthy provider. Seeded disabled instead, so it appears in the
# UI and can be flipped on after `grok` is signed in.
grok_auth = (
    pathlib.Path(os.path.expanduser("~/.grok/auth.json")).is_file()
    or bool(env("GROK_AUTH"))
    or bool(env("GROK_API_KEY"))
)
desired["grok"] = {
    "driver": "grok",
    "displayName": "Grok" if grok_auth else "Grok (no credentials)",
    "enabled": grok_auth,
}

# Instances that need an explicit binary, pulled from the unit environment so
# the store paths are never hard-coded here.
for instance_id, display, var in (
    ("opencode-v2", "opencode v2", "OPENCODE_V2_BIN"),
    ("antigravity-acp", "Antigravity (ACP)", "ANTIGRAVITY_ACP_BIN"),
):
    binary = env(var)
    if not binary:
        continue
    driver = "opencode" if instance_id.startswith("opencode") else "antigravity"
    desired[instance_id] = {
        "driver": driver,
        "displayName": display,
        "enabled": True,
        "config": (
            {"binaryPath": binary, "model": OPENCODE_MODEL}
            if driver == "opencode"
            else {"binaryPath": binary}
        ),
    }

instances = doc.setdefault("providerInstances", {})
added = [k for k in desired if k not in instances]
for k in added:
    instances[k] = desired[k]

if not added:
    sys.exit(0)

tmp = path.with_suffix(".seed-tmp")
tmp.write_text(json.dumps(doc, indent=2) + "\n")
tmp.replace(path)
print("t3code-seed-providers: added " + ", ".join(sorted(added)), file=sys.stderr)
PY
