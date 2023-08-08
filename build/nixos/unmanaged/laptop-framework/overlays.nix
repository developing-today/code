{ pkgs, ... }: {
  nixpkgs.overlays = [ pkgs.zig.overlays.default ];
}
