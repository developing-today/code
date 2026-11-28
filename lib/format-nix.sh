#!/usr/bin/env bash

find . -name '*.nix' | xargs nixfmt -s -v
