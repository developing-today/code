{
  inputs.nixpkgs.url = "nixpkgs/nixos-unstable";
  inputs.zig-overlay.url = "github:mitchellh/zig-overlay";

  outputs = { self, nixpkgs, zig-overlay, ... }: {
    nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        ./overlays.nix
      ];
    };
  };
}
