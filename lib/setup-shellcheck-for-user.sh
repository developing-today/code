#!/usr/bin/env bash

printf "%s\n" "start: setup-shellcheck-for-user script"

SAVED_SHELL_OPTIONS=$(set +o)

# shellcheck disable=SC2317
restore_shell_options() {
  printf "%s\n" "trap start: restoring shell options"
  eval "${SAVED_SHELL_OPTIONS}"
  printf "%s\n" "trap done: restoring shell options"
}
trap restore_shell_options EXIT

set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

create_symlink_for_shellcheckrc() {
    local user home_dir target_file source_file

    user="$1"

    # Get home directory for the given user
    if ! home_dir=$(eval echo ~"$user"); then
        echo "Error: Failed to retrieve home directory for user $user." >&2
        exit 1
    fi

    target_file="$home_dir/.shellcheckrc"
    source_file="$(dirname "$(realpath "$0")")/.shellcheckrc"

    # Check if the target file exists
    if [[ -e "$target_file" ]]; then
        # If the target is a symlink
        if [[ -L "$target_file" ]]; then
            # If it doesn't point to the source file, panic!
            if [[ "$(readlink "$target_file")" != "$source_file" ]]; then
                echo "Error: $target_file is a symlink, but not to $source_file!" >&2
                exit 1
            fi
        else
            echo "Error: $target_file exists and is not a symlink!" >&2
            exit 1
        fi
    else
        # Create a symlink
        printf "symlink: %s %s\n" "$source_file" "$target_file"
        ln -s "$source_file" "$target_file"
    fi
}

if [[ "$#" -eq 0 ]]; then
    # No arguments, just run for the current user
    create_symlink_for_shellcheckrc "$USER"
else
    for user in "$@"; do
        if [[ "$user" != "$USER" && "$EUID" -ne 0 ]]; then
            # If the user is not the current user and the script is not running as root, rerun with sudo
            sudo "$0" "$user"
        else
            create_symlink_for_shellcheckrc "$user"
        fi
    done
fi

printf "%s\n" "done: setup-shellcheck-for-user script"
