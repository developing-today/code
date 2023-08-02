{
  description = "tailwindcss-cli";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
  };

  outputs = { self, nixpkgs }: {
    defaultPackage.x86_64-linux =
      with import nixpkgs { system = "x86_64-linux"; };
        import ./tailwindcss.nix { pkgs = pkgs; };
  };
}
