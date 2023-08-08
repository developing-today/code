{ zig-overlay, ... }: {
  nixpkgs.overlays = [ zig-overlay.overlays.default ];
}
