#!/usr/bin/env bash

# nix-shell

./db-start.sh

# trunk serve

cargo leptos watch
