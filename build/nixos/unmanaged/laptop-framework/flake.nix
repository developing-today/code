{
  inputs.nixpkgs.url = "nixpkgs/nixos-unstable";
  inputs.zig.url = "github:mitchellh/zig-overlay";

  outputs = { self, nixpkgs, zig, ... }: {
    nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        { pkgs, ... }: { 
          imports = [ ./configuration.nix ];
          nixpkgs.overlays = [ zig.overlays.default ];
        }
      ];
    };
  };
}
