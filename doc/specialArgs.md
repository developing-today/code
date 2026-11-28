let depInject = { pkgs, lib, ... }: {
options.dep-inject = lib.mkOption {
type = with lib.types; attrsOf unspecified;
default = { };
};
config.dep-inject = { # inputs comes from the outer environment of flake.nix
flake-inputs = inputs;
};
};
in {
nixosModules.default = { pkgs, lib, ... }: {
imports = [ depInject ];
};
}
