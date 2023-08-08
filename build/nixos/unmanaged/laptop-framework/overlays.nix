{ nixpkgs, ... }: {
  nixpkgs.overlays = [ nixpkgs.zig-overlay.defaultPackage ];
}
