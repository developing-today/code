#!/usr/bin/env bash
set -e #-o pipefail

echo "Getting user"
my_user=$(whoami)
echo "you are user: $my_user"

echo "Forcing permissions for .git"
# :users
echo "sudo chown -R \"$my_user\" .git"
sudo chown -R "$my_user" .git
echo "done forcing permissions"
