{ nixpkgs, zig-overlay, ... }: {
  nixpkgs.overlays = [ zig-overlay.defaultPackage ];
}
