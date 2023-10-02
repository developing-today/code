{
  # Import all your configuration modules here
  imports = map (n: ./modules + "/${n}") (builtins.filter (n: builtins.match ".*\\.nix" n != null) (builtins.attrNames (builtins.readDir ./modules)));
}
