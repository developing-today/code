# openai-cred-watchdog — hourly check that the laptop OpenCode
# credential store keeps the invariant whose violation caused
# developing-today/code#3216 (2026-10-31): a chatgpt-token-sharing
# credential for provider `openai` became the active one while holding
# a Codex-flow token, and every fleet session reaching the GPT leg
# failed with 401 "Missing scopes: api.responses.write" for ~4 hours.
#
# Drew (2026-10-31): "all services and binaries should be in code
# repo" — this module relocates the watchdog from its interim home
# (~/hatch/tools/openai-cred-watchdog.py in dezren39/hatch) into this
# flake, on the pattern of the laptop user services already here
# (quota-gatherer in nixos/services/pm-stack; the watchdogs in
# nixos/home): the script is vendored beside the module, packaged as
# a store binary, and run by a oneshot user service on a user timer.
#
# The check is read-only and local — one sqlite read of the store at
# ~/.local/share/opencode/opencode.db — and prints credential
# metadata only (ids, labels, methods; never token values). Exit 0
# healthy, 1 on violation, 2 when the store cannot be read. There is
# no Drew-visible alert surface for user-unit failures beyond the
# failed unit + journal in this repo's current shape — the same surface
# the laptop-backups and codex-remote-control watchdogs rely on — so
# v1 surfaces a violation exactly that way: this unit fails, naming
# the offending credential in the journal.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.openai-cred-watchdog;
  home = "/home/${cfg.user}";
  # The script is vendored verbatim from its interim home; the
  # writer lint is muted for its shebang (E265) and its long string
  # literals (E501) — the OK/VIOLATION line texts are the contract.
  watchdogPkg = pkgs.writers.writePython3Bin "openai-cred-watchdog" {
    flakeIgnore = [
      "E265"
      "E501"
    ];
  } (builtins.readFile ./openai-cred-watchdog.py);
in
{
  options.services.openai-cred-watchdog = {
    enable = lib.mkEnableOption "the OpenAI credential watchdog (hourly invariant check of the OpenCode credential store, code#3216)";

    user = lib.mkOption {
      type = lib.types.str;
      default = "user";
      description = "The user whose home-manager session runs the watchdog; the credential store lives in this user home.";
    };
  };

  config = lib.mkIf cfg.enable {
    home-manager.users.${cfg.user} = {
      systemd.user.services.openai-cred-watchdog = {
        Unit.Description = "OpenAI credential watchdog — assert no token-sharing credential serves the OpenCode GPT providers (code#3216)";
        Service = {
          Type = "oneshot";
          ExecStart = "${watchdogPkg}/bin/openai-cred-watchdog ${home}/.local/share/opencode/opencode.db";
        };
      };

      systemd.user.timers.openai-cred-watchdog = {
        Unit.Description = "Run the OpenAI credential watchdog hourly (code#3216)";
        Timer = {
          OnCalendar = "hourly";
          RandomizedDelaySec = 20;
          Persistent = true;
          Unit = "openai-cred-watchdog.service";
        };
        Install.WantedBy = [ "timers.target" ];
      };
    };
  };
}
