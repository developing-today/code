# nix/nixpkgs-inputs.nix
#
# Parse flake.lock to find direct root inputs whose original source is NixOS/nixpkgs.
# Returns a newline-separated string of input names, optionally filtered by ref (branch).
#
# Usage:
#   nix eval --raw --file nix/nixpkgs-inputs.nix
#   nix eval --raw --file nix/nixpkgs-inputs.nix --argstr ref master
#   nix eval --raw --file nix/nixpkgs-inputs.nix --argstr ref nixos-unstable
#
# ref = null means all NixOS/nixpkgs inputs regardless of branch.
# ref = "master" matches inputs with ref "master" or no ref (default branch is master).
# ref = "nixos-unstable" matches inputs with ref "nixos-unstable".

{
  ref ? null,
}:

let
  lock = builtins.fromJSON (builtins.readFile ../flake.lock);
  rootInputs = lock.nodes.root.inputs;

  # Get the node for a root input. Skip follows (array values).
  directInputs = builtins.filter (entry: entry != null) (
    builtins.attrValues (
      builtins.mapAttrs (
        name: nodeName:
        if builtins.isList nodeName then
          null # follows — not a direct input
        else
          {
            inherit name;
            node = lock.nodes.${nodeName};
          }
      ) rootInputs
    )
  );

  # Filter to NixOS/nixpkgs inputs
  isNixOSNixpkgs =
    entry:
    let
      orig = entry.node.original or { };
    in
    (orig.type or "" == "github")
    && (builtins.match "[Nn]ix[Oo][Ss]" (orig.owner or "") != null)
    && (orig.repo or "" == "nixpkgs");

  nixpkgsInputs = builtins.filter isNixOSNixpkgs directInputs;

  # Filter by ref if specified
  matchesRef =
    entry:
    let
      orig = entry.node.original or { };
      inputRef = orig.ref or null;
    in
    if ref == null then
      true
    else if ref == "master" then
      # master matches both explicit "master" and no ref (github default)
      inputRef == null || inputRef == "master"
    else
      inputRef == ref;

  filtered = builtins.filter matchesRef nixpkgsInputs;
  names = map (entry: entry.name) filtered;
in
builtins.concatStringsSep "\n" names
