#!/bin/bash

git submodule update --init --recursive

curl -L https://nixos.org/nix/install | sh -s -- --daemon

. /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh

/etc/profiles/per-user/$USER/etc/profile.d/hm-session-vars.sh

USER_NAME=$USER
sudo mkdir -m 0755 -p /nix/var/nix/{profiles,gcroots}/per-user/$USER_NAME
sudo chown -R $USER_NAME:$USER_NAME /nix/var/nix/{profiles,gcroots}/per-user/$USER_NAME

echo "trusted-users = root $USER_NAME" | sudo tee -a /etc/nix/nix.conf
echo "allowed-users = root $USER_NAME" | sudo tee -a /etc/nix/nix.conf
echo 'experimental-features = flakes nix-command ca-derivations' | sudo tee -a /etc/nix/nix.conf

nix-env -i direnv
echo "eval \"\$(direnv hook bash)\"" >> ~/.profile
eval "$(direnv hook bash)"

# exit; wsl --shutdown; ubuntu

nix run \
    --tarball-ttl 0 \
    --accept-flake-config \
    'github:thoughtpolice/buck2-nix?dir=buck/nix#setup'

cd $HOME/buck2-nix.sl

buck2 build ...
