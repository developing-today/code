#!/usr/bin/env bash

SAVED_SHELL_OPTIONS=$(set +o)

# shellcheck disable=SC2317
restore_shell_options() {
  printf "%s\n" "trap start: restoring shell options"
  eval "${SAVED_SHELL_OPTIONS}"
  printf "%s\n" "trap done: restoring shell options"
}
trap restore_shell_options EXIT

set -euo pipefail

create_xinitrc() {
  local user home_dir target_file

  user="$1"

  if ! home_dir=$(eval echo ~"$user"); then
    printf "Error: Failed to retrieve home directory for user %s.\n" "$user" >&2
    exit 1
  fi
  
  if [[ ! -d "${home_dir}" ]]; then
    echo "Directory ${home_dir} does not exist"
    exit 1
  fi

  target_file="$home_dir/.xinitrc"

  if [[ -e "$target_file" ]]; then
    printf "Error: %s file already exists\n" "$target_file" >&2
    exit 1
  else
    printf "#!/usr/bin/env bash\n\n# Remap Caps Lock to Escape\nsetxkbmap -option caps:escape\n" > "$target_file"
    chmod +x "$target_file"
  fi
}

if [[ "$#" -eq 0 ]]; then
  create_xinitrc "$USER"
else
  for user in "$@"; do
    if [[ "$user" != "$USER" && "$EUID" -ne 0 ]]; then
      sudo "$0" "$user"
    else
      create_xinitrc "$user"
    fi
  done
fi

printf "%s\n" "done: setup-xinitrc-for-user script"

