{
  description = "openclaw plugin: summarize (vendored from openclaw/nix-openclaw-tools)";

  # Vendored because the bundled plugin's pnpm deps hash, pinned in
  # nix-openclaw-tools at the rev hardcoded in nix-openclaw's module
  # (e1a2eb89), does not reproduce under that repo's own locked toolchain
  # in this build environment: the fixed-output fetch produces
  # sha256-55jIaqsVM6elegihK16TWxj7BRHb8kqvHEHw4fRZjPE= instead of the
  # pinned sha256-wj/XNTJHSgINkL3lMNrhurFdfAUavaVargB3w7+UP88=, which
  # broke the laptop NixOS switch (hash mismatch in summarize-pnpm-deps,
  # 2026-10-30). Here the package is the upstream expression verbatim,
  # the deps hash is pinned under THIS flake's locked nixpkgs, and this
  # repo owns the pin. Re-lock deliberately when nixpkgs moves.
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/6510408d147d8c5e6d76043e899e9251756f6a75";
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
