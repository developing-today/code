{ zig, ... }: {
  nixpkgs.overlays = [ zig.overlays.default ];
}
