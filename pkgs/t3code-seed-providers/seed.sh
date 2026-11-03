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
# Existing entries are mostly left alone -- this adds instances that are
# absent, so anything configured in the UI survives. Two deliberate exceptions,
# both scoped to fleet-owned instances / fleet-known hazards:
#   * Refresh-on-seed: for the fleet-owned instance ids below, a stale
#     config.binaryPath is updated in place when the unit environment provides
#     a different binary. The seed was add-only originally, so store-path bumps
#     stranded stale binaries forever -- the live settings still pointed
#     opencode-v2 at a 2.0.23 build long after the fleet moved on.
#   * Safety disable: any existing opencode-driver instance pointing at an
#     OpenCode 1.x binary is disabled (see the V2 note in the Python below).

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
except (OSError, ValueError) as exc:
    print(f"t3code-seed-providers: cannot read settings: {exc}", file=sys.stderr)
    sys.exit(0)  # don't clobber something we can't parse

if not isinstance(doc, dict):
    print("t3code-seed-providers: settings must be an object; leaving unchanged", file=sys.stderr)
    sys.exit(0)
if "providerInstances" in doc and not isinstance(doc["providerInstances"], dict):
    print("t3code-seed-providers: providerInstances must be an object; leaving unchanged", file=sys.stderr)
    sys.exit(0)

def env(name):
    v = os.environ.get(name, "").strip()
    return v or None

# Default model per instance (T3 Code 0.1.2, Orchestrator V2).
#
# The V1 line's built-in DEFAULT_MODEL_BY_PROVIDER was:
#   codex -> gpt-6-astra, claudeAgent -> claude-fable-5-1, cursor -> auto,
#   grok -> grok-build, opencode -> openai/gpt-5, antigravity -> antigravity-default
#
# gpt-5 and gpt-6-astra are both superseded; Luna is still the fleet default
# (see the route pools in opencode.jsonc), so the explicit Luna pins for
# codex / opencode below are kept. Set explicitly so new instances do not
# land on a stale upstream default.
#
# claudeAgent is deliberately NOT given a Luna model: that driver runs Claude
# Code, which can only serve Anthropic models. Pointing it at an OpenAI slug
# would just break it. Its "fable" default is left alone. The muse and pi
# instances likewise get no model pin: muse serves Meta models through its
# own CLI, and pi resolves models through its own config.
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
# the store paths are never hard-coded here. Skipped entirely when the env
# var is unset, so a host without that provider gets no broken instance.
#
# "muse" is T3 0.1.2's first-class Muse Code driver id (literally "muse";
# the driver looks for the `muse` CLI and its own sign-in on this host).
for instance_id, driver, display, var in (
    ("opencode-v2", "opencode", "opencode v2", "OPENCODE_V2_BIN"),
    ("antigravity-acp", "antigravity", "Antigravity (ACP)", "ANTIGRAVITY_ACP_BIN"),
    ("muse", "muse", "Muse Code", "MUSE_BIN"),
):
    binary = env(var)
    if not binary:
        continue
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

# Pi, like Grok above, is only enabled when credentials are actually present
# (~/.pi/agent/auth.json, written by `pi`'s own login). With none, an enabled
# instance just shows up as a permanently unhealthy provider, so it is seeded
# disabled instead and can be flipped on in the UI after signing in.
pi_binary = env("PI_BIN")
if pi_binary:
    pi_auth = pathlib.Path(os.path.expanduser("~/.pi/agent/auth.json")).is_file()
    desired["pi"] = {
        "driver": "pi",
        "displayName": "Pi" if pi_auth else "Pi (no credentials)",
        "enabled": pi_auth,
        "config": {"binaryPath": pi_binary},
    }

instances = doc.setdefault("providerInstances", {})
added = [k for k in desired if k not in instances]
for k in added:
    instances[k] = desired[k]

# Refresh-on-seed for the fleet-owned instances: if the instance already
# exists and the environment provides a binary path that differs from the
# stored config.binaryPath, update binaryPath in place -- preserving
# displayName, enabled and everything else the UI may have set. Without this,
# every store-path bump strands the old binary in settings.json forever.
FLEET_OWNED = ("opencode-v2", "antigravity-acp", "muse", "pi")
malformed = set()
for k in FLEET_OWNED:
    if k not in instances:
        continue
    inst = instances[k]
    if not isinstance(inst, dict) or ("config" in inst and not isinstance(inst["config"], dict)):
        print(f"t3code-seed-providers: skipping malformed fleet-owned instance {k}", file=sys.stderr)
        malformed.add(k)
refreshed = []
for k in FLEET_OWNED:
    if k in added or k not in instances or k not in desired or k in malformed:
        continue
    binary = (desired[k].get("config") or {}).get("binaryPath")
    if not binary:
        continue
    config = instances[k].setdefault("config", {})
    if config.get("binaryPath") != binary:
        config["binaryPath"] = binary
        refreshed.append(k)

# Safety: T3 Code V2 must never drive an OpenCode 1.x binary against the
# shared OpenCode 2.x session DB (~/.local/share/opencode) -- OpenChamber and
# the opencode-v2 instance both use that DB with 2.x binaries only. Any
# existing opencode-driver instance pointing at a 1.x build is disabled here;
# that is reversible in the UI, but it should not be re-enabled.
disabled = []
for k, inst in instances.items():
    if k in malformed or not isinstance(inst, dict) or inst.get("driver") != "opencode":
        continue
    config = inst.get("config", {})
    if not isinstance(config, dict):
        continue
    binary = config.get("binaryPath") or ""
    if not isinstance(binary, str):
        continue
    # Matches both an opencode-1.x directory ("/opencode-1.") and a Nix store
    # path segment ("<hash>-opencode-1.").
    if ("/opencode-1." in binary or "-opencode-1." in binary) and inst.get("enabled") is not False:
        inst["enabled"] = False
        disabled.append(k)

if not added and not refreshed and not disabled:
    sys.exit(0)

tmp = path.with_suffix(".seed-tmp")
tmp.write_text(json.dumps(doc, indent=2) + "\n")
tmp.replace(path)
changes = []
if added:
    changes.append("added " + ", ".join(sorted(added)))
if refreshed:
    changes.append("refreshed binaryPath for " + ", ".join(sorted(refreshed)))
if disabled:
    changes.append(
        "disabled OpenCode 1.x instance(s) " + ", ".join(sorted(disabled))
        + " (V2 must never drive a 1.x binary against the shared 2.x session DB)"
    )
print("t3code-seed-providers: " + "; ".join(changes), file=sys.stderr)
PY
