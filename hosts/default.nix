inputs:
let
  lib = inputs.self.lib;
in
  lib.make-nixos-hosts inputs.self.hosts
