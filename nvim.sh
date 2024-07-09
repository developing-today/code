AUTH=$(cat /home/user/auth)
NIX_CONFIG="access-tokens = github.com=${AUTH}" nix run github:developing-today/code?dir=build/nixos/unmanaged/laptop-framework/flakes/home/programs/nixvim
