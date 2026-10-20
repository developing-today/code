{
  lib,
  stdenvNoCC,
  bash,
  coreutils,
  gnugrep,
}:

# agent-shell-env -- makes bws, 1Password and Jules auth part of every hidden shell.
#
#   share/agent-shell-env/env.sh  hook; BASH_ENV points here (nixos/agent-auth,
#                                 home/common) and ~/.bashrc sources it
#   bin/agent-auth                status / login / env helper
#   bin/op-personal               `op` without the service-account token, so it
#                                 falls back to the desktop app (personal vaults)
#
# The store only ever holds file PATHS; secret values are read at shell start
# from sops (/run/secrets/...) or 0600 files under ~/.config. See env.sh.
stdenvNoCC.mkDerivation {
  pname = "agent-shell-env";
  version = "1";

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./env.sh
      ./agent-auth.sh
    ];
  };

  buildInputs = [ bash ]; # patchShebangs target for agent-auth
  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    runHook preInstall

    install -Dm444 env.sh "$out/share/agent-shell-env/env.sh"
    install -Dm555 agent-auth.sh "$out/bin/agent-auth"

    cat > "$out/bin/op-personal" <<EOF
    #!${bash}/bin/bash
    # 1Password CLI via the desktop app (all vaults) instead of the service account.
    exec env -u OP_SERVICE_ACCOUNT_TOKEN op "\$@"
    EOF
    chmod 555 "$out/bin/op-personal"

    substituteInPlace "$out/share/agent-shell-env/env.sh" "$out/bin/agent-auth" \
      --subst-var out
    substituteInPlace "$out/bin/agent-auth" \
      --replace-fail "@toolsPath@" "${
        lib.makeBinPath [
          coreutils
          gnugrep
        ]
      }"

    runHook postInstall
  '';

  doInstallCheck = true;
  installCheckPhase = ''
    # Hook must be silent, must not fail under `set -eu`, and must not leak helpers.
    out_text="$(HOME="$TMPDIR" ${bash}/bin/bash -euc '. "$0"; declare -F __ase_load && exit 1; echo "$BASH_ENV"' "$out/share/agent-shell-env/env.sh")"
    [ "$out_text" = "$out/share/agent-shell-env/env.sh" ]
    mkdir -p "$TMPDIR/.config/jules"
    printf 'k123\n' > "$TMPDIR/.config/jules/api-key"
    [ "$(HOME="$TMPDIR" BASH_ENV="$out/share/agent-shell-env/env.sh" ${bash}/bin/bash -c 'echo "$JULES_API_KEY"')" = k123 ]
    [ "$(HOME="$TMPDIR" JULES_API_KEY=pre BASH_ENV="$out/share/agent-shell-env/env.sh" ${bash}/bin/bash -c 'echo "$JULES_API_KEY"')" = pre ]
    mkdir -p "$TMPDIR/.config/bws"
    printf 'bws-tok\n' > "$TMPDIR/.config/bws/access-token"
    [ "$(HOME="$TMPDIR" BASH_ENV="$out/share/agent-shell-env/env.sh" ${bash}/bin/bash -c 'echo "$BWS_ACCESS_TOKEN"')" = bws-tok ]
    [ -z "$(HOME="$TMPDIR" BASH_ENV="$out/share/agent-shell-env/env.sh" ${bash}/bin/bash -c 'true' 2>&1)" ]
  '';

  meta = {
    description = "Auth environment (bws, 1Password service account, Jules) for every non-interactive shell";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "agent-auth";
  };
}
