# Codespaces/devcontainer image for developing-today/code.
# The repo's real environment is its Nix flakes (.envrc: `use flake` +
# `use flake ./src/id`), so the image stays deliberately thin: Ubuntu base,
# direnv, and a single-user Nix install with flakes on. Everything else
# comes from the flakes, same as on the laptop.
FROM mcr.microsoft.com/devcontainers/base:ubuntu-24.04

USER root
RUN apt-get update \
 && apt-get install -y --no-install-recommends direnv \
 && rm -rf /var/lib/apt/lists/*

USER vscode
# Single-user (no-daemon) Nix install for the vscode user.
RUN curl -L https://nixos.org/nix/install | sh -s -- --no-daemon \
 && mkdir -p "$HOME/.config/nix" \
 && printf 'experimental-features = nix-command flakes\nbuild-users-group =\n' > "$HOME/.config/nix/nix.conf"

ENV PATH="/home/vscode/.nix-profile/bin:${PATH}"
