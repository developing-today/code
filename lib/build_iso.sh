#!/usr/bin/env bash

set -e

if [ "$EUID" -ne 0 ]; then
  echo "Re-executing with sudo"
  exec sudo "$0" "$@"
fi

get_script_dir() {
    SOURCE="${BASH_SOURCE[0]}"
    while [ -h "$SOURCE" ]; do
        DIR="$( cd -P "$( dirname "$SOURCE" )" && pwd )"
        SOURCE="$(readlink "$SOURCE")"
        [[ $SOURCE != /* ]] && SOURCE="$DIR/$SOURCE"
    done
    echo "$( cd -P "$( dirname "$SOURCE" )" && pwd )"
}

find_repo_root() {
    local dir="$1"
    while [ "$dir" != "/" ]; do
        if [ -d "$dir/.git" ]; then
            echo "$dir"
            return 0
        fi
        dir="$(dirname "$dir")"
    done
    echo "Error: Not in a Git repository" >&2
    return 1
}

script_dir=$(get_script_dir)
repo_root=$(find_repo_root "$script_dir")

if [ $? -ne 0 ]; then
    exit 1
fi

echo "Changing to repository root: $repo_root"
cd "$repo_root"

hostname=$(hostname)
bootstrap=false

if [ "$#" -eq 0 ]; then
    echo "No arguments provided. Using system hostname: $hostname"
elif [ "$#" -eq 1 ]; then
    if [ "$1" = "-bootstrap" ]; then
        bootstrap=true
        echo "Bootstrap flag detected. Using system hostname: $hostname"
    else
        hostname="$1"
        echo "Using provided hostname: $hostname"
    fi
elif [ "$#" -eq 2 ] && [ "$2" = "-bootstrap" ]; then
    hostname="$1"
    bootstrap=true
    echo "Using provided hostname: $hostname with bootstrap option"
else
    echo "Error: Invalid arguments."
    echo "Usage: $0 [hostname] [-bootstrap]"
    exit 1
fi

echo "Building ISO for hostname: $hostname"

nix build ".#nixosConfigurations.\"unattended-installer_$hostname\".config.system.build.isoImage"

if [ $? -ne 0 ]; then
    echo "Error: Nix build failed."
    exit 1
fi

echo "Nix build completed successfully."

if [ "$bootstrap" = true ]; then
    bootstrap_script="${script_dir}/bootstrap_iso.sh" "$hostname"

    if [ -f "$bootstrap_script" ]; then
        echo "Executing bootstrap script: $bootstrap_script"
        chmod +x "$bootstrap_script"
        "$bootstrap_script"

        if [ $? -ne 0 ]; then
            echo "Error: Bootstrap script execution failed."
            exit 1
        fi
    else
        echo "Warning: Bootstrap script not found at $bootstrap_script"
    fi
fi

echo "ISO build process completed successfully!"
