{ nixpkgs, zig, ... }: {
  nixpkgs.overlays = [ zig.overlay ];
}
