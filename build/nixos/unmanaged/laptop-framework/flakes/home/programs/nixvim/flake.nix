{
  inputs = {
    #     nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable"; # /nixos-23.11";
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"; # /nixos-unstable"; # /nixos-23.11";
    nixvim-upstream = {
      url = "github:nix-community/nixvim";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
      inputs.beautysh.follows = "beautysh";
      inputs.pre-commit-hooks.follows = "pre-commit-hooks";
    };
    flake-utils.url = "https://flakehub.com/f/numtide/flake-utils/*.tar.gz"; #*/ # inputs.systems

    beautysh = {
      url = "github:lovesegfault/beautysh";
      inputs.nixpkgs.follows = "nixpkgs";
      # todo: drag out these follows
    };

    pre-commit-hooks = {
      url = "github:cachix/pre-commit-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
      # todo: drag out these follows
    };
  };
  outputs = {
    nixpkgs,
    nixvim-upstream,
    flake-utils,
    ...
  } @ inputs: let
    config = import ./config; # import the module directly
  in
    flake-utils.lib.eachDefaultSystem (system: let
      nixvimLib = nixvim-upstream.lib.${system};
      pkgs = import nixpkgs {inherit system;};
      nixvim = nixvim-upstream.legacyPackages.${system};
      nixosModules = nixvim-upstream.nixosModules.nixvim;
      homeManagerModules = nixvim-upstream.homeManagerModules.nixvim;
      nvim = nixvim-upstream.makeNixvimWithModule {
        inherit pkgs;
        module = config;
      };
    in {
      packages = {
        default = nvim;
      };
      nixosModules = nixosModules; # Re-export from nixvim
      homeManagerModules = homeManagerModules; # Re-export from nixvim
    });
}
