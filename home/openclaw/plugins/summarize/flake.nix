{
  description = "openclaw plugin: summarize (vendored from openclaw/nix-openclaw-tools)";

  # Vendored from openclaw/nix-openclaw-tools (rev e1a2eb89,
  # hardcoded in nix-openclaw's module). The pnpm deps hash in
  # summarize.nix is the value the upstream toolchain actually
  # produces -- sha256-55jIaqsVM6elegihK16TWxj7BRHb8kqvHEHw4fRZjPE=,
  # taken from the 2026-10-30 switch error -- so this flake locks
  # nixpkgs to the tools repo's own locked rev (aff8a0b2,
  # pnpm 11.25.0) to reproduce that output exactly. Upstream's
  # summarize.nix pins sha256-wj/XNTJHSgINkL3lMNrhurFdfAUavaVargB3w7+UP88=;
  # do not change the pin without changing this lock to match.
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/aff8a0b28396750446e5537a96461bc4facdb287";
  };

  outputs =
    { self, nixpkgs }:
    let
      lib = nixpkgs.lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      summarizeFor =
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        pkgs.callPackage ./summarize.nix {
          pnpm = pkgs.pnpm_11;
          nodejs = pkgs.nodejs_24;
        };
      pluginFor =
        system:
        let
          summarize = summarizeFor system;
        in
        {
          name = "summarize";
          skills = [ ./skills/summarize ];
          packages = [ summarize ];
          needs = {
            stateDirs = [ ];
            requiredEnv = [ ];
          };
        };
    in
    {
      packages = lib.genAttrs systems (system: {
        summarize = summarizeFor system;
      });

      openclawPlugin = pluginFor;
    };
}
