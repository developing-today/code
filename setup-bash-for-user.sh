#!/usr/bin/env bash

printf "%s\n" "start: setup-bash-for-user script"

SAVED_SHELL_OPTIONS=$(set +o)

# shellcheck disable=SC2317
restore_shell_options() {
  printf "%s\n" "trap start: restoring shell options"
  # printf "%s\n" "SAVED_SHELL_OPTIONS: ${SAVED_SHELL_OPTIONS}"
  # printf "%s\n" "CURRENT_SHELL_OPTIONS: $(set +o)"

  eval "${SAVED_SHELL_OPTIONS}"

  printf "%s\n" "trap done: restoring shell options"
}
trap restore_shell_options EXIT
set -eu # pipefail causes issues ?

source ./export-lib.sh

# System-wide content for .bash_profile can be added here (for all users specified)
# MUST ESCAPE DOUBLE QUOTES WITHIN CONTENT
mapfile -t bash_profile_global_content_lines << EOF
# ============================================================== START OF AUTOMATICALLY GENERATED CODE ==============================================================
# NOTE: THIS FILE AUTOMATICALLY GENERATED BY 'setup-bash-for-user.sh'.
# DO NOT CHANGE ANYTHING ABOVE LINE STARTING WITH: '# ============================================================== END OF AUTOMATICALLY GENERATED CODE =============================================================='
# ============================================================== START OF SYSTEM-WIDE GENERATED CODE ==============================================================

# System-wide content for .bash_profile can be added below here:
alias c='nix-shell --command \"code-insiders .\"'
alias cn='code-insiders .'
alias l='nix-shell --command \"cargo leptos watch\"'
alias n='nix-shell'
alias vim='nvim'
alias z='nix-shell --command \"\\\$(fc -ln -1)\"'
STARSHIP_PATH=\"\\\$(which starship)\" # do not use realpath
eval \"\\\$(\\\$STARSHIP_PATH init bash)\"
# ============================================================== END OF SYSTEM-WIDE GENERATED CODE ==============================================================
EOF
bash_profile_global_content=$(printf "%s\n" "${bash_profile_global_content_lines[@]}")

# System-wide content for .bashrc can be added here (for all users specified)
# MUST ESCAPE DOUBLE QUOTES WITHIN CONTENT
mapfile -t bashrc_global_content_lines << EOF
# ============================================================== START OF AUTOMATICALLY GENERATED CODE ==============================================================
# NOTE: THIS FILE AUTOMATICALLY GENERATED BY 'setup-bash-for-user.sh'.
# DO NOT CHANGE ANYTHING ABOVE LINE STARTING WITH: '# ============================================================== END OF AUTOMATICALLY GENERATED CODE =============================================================='
# ============================================================== START OF SYSTEM-WIDE GENERATED CODE ==============================================================

# System-wide content for .bashrc can be added below here:
[[ -f ~/.bash_profile ]] && . ~/.bash_profile

# ============================================================== END OF SYSTEM-WIDE GENERATED CODE ==============================================================
EOF
bashrc_global_content=$(printf "%s\n" "${bashrc_global_content_lines[@]}")

# TODO: USE LOCAL VARIABLES
# shellcheck disable=SC2317
process_user() {
  username="${1}"
  is_force="${2}"

  if ! id -u "$username" >/dev/null 2>&1; then
    printf "%s\n" "is error: id does not exist for user: '$username'"

    return 1
  fi

  user_home=$(getent passwd "$username" | cut -d: -f6)

  # Per user content for .bash_profile can be added here using mapfile (inside user-specific loop)
  # MUST ESCAPE DOUBLE QUOTES WITHIN CONTENT
  mapfile -t bash_profile_user_content_lines << EOF
# THIS FILE AUTOMATICALLY GENERATED BY 'setup-bash-for-user.sh'.
# DO NOT CHANGE ANYTHING ABOVE LINE STARTING WITH: '# ============================================================== END OF AUTOMATICALLY GENERATED CODE =============================================================='
# ============================================================== START OF PER-USER GENERATED CODE ==============================================================

# Per user content for .bash_profile can be added below here:

# ============================================================== END OF PER-USER GENERATED CODE ==============================================================
# NOTE: Only make manual changes after the following line containing the END banner:
# ============================================================== END OF AUTOMATICALLY GENERATED CODE ==============================================================
EOF
  bash_profile_user_content=$(printf "%s\n" "${bash_profile_user_content_lines[@]}")

  # Per user content for .bashrc can be added here using mapfile (inside user-specific loop)
  # MUST ESCAPE DOUBLE QUOTES WITHIN CONTENT
  mapfile -t bashrc_user_content_lines << EOF
# THIS FILE AUTOMATICALLY GENERATED BY 'setup-bash-for-user.sh'.
# DO NOT CHANGE ANYTHING ABOVE LINE STARTING WITH: '# ============================================================== END OF AUTOMATICALLY GENERATED CODE =============================================================='
# ============================================================== START OF PER-USER GENERATED CODE ==============================================================

# Per user content for .bashrc can be added below here:

# ============================================================== END OF PER-USER GENERATED CODE ==============================================================
# NOTE: Only make manual changes after the following line containing the END banner:
# ============================================================== END OF AUTOMATICALLY GENERATED CODE ==============================================================
EOF
  bashrc_user_content=$(printf "%s\n" "${bashrc_user_content_lines[@]}")

  # Combined content for .bash_profile and .bashrc
  combined_bash_profile_content=$(printf "%s\n%s\n" "$bash_profile_global_content" "$bash_profile_user_content")
  combined_bashrc_content=$(printf "%s\n%s\n" "$bashrc_global_content" "$bashrc_user_content")

  # Create temporary files for check and push scripts
  check_script_file=$(mktemp)
  push_script_file=$(mktemp)

  # Create temporary files for expected content
  expected_bash_profile_file=$(mktemp)
  expected_bashrc_file=$(mktemp)

  # Define cleanup function
  # shellcheck disable=SC2317
  cleanup() {
    printf "%s\n" "trap start: cleanup temp files"

    rm -f "$check_script_file" "$push_script_file" "$expected_bash_profile_file" "$expected_bashrc_file"

    printf "%s\n" "trap done: cleanup temp files"
  }
  trap cleanup EXIT

  # Create push script content
  mapfile -t push_script_content_lines << EOF
printf "%s\n" "start: push bash script for user: '$username'"
printf "%s\n" "update: .bash_profile for user: '$username'"
printf "%s\n" "$combined_bash_profile_content" > "$user_home/.bash_profile"
printf "%s\n" "chown: .bash_profile for user: '$username'"

chown "$username" "$user_home/.bash_profile"

printf "%s\n" "chmod: .bash_profile for user: '$username'"

chmod 644 "$user_home/.bash_profile"

printf "%s\n" "update: .bashrc for user: '$username'"
printf "%s\n" "$combined_bashrc_content" > "$user_home/.bashrc"
printf "%s\n" "chown: .bashrc for user: '$username'"

chown "$username" "$user_home/.bashrc"

printf "%s\n" "chmod: .bashrc for user: '$username'"

chmod 644 "$user_home/.bashrc"

printf "%s\n" "done: push bash script for user: '$username'"
EOF
  printf "%s\n" "${push_script_content_lines[@]}" > "$push_script_file"

  # Create check script content
  mapfile -t check_script_content_lines << EOF
printf "%s\n" "start: dotfile check for user: '$username'"

is_run_push_script=true
is_error=false

printf "%s\n" "checking: .bash_profile for user: '$username'"

if [ -f "$user_home/.bash_profile" ]; then
  printf "%s\n" "$combined_bash_profile_content" > "$expected_bash_profile_file"

  if diff -q "$expected_bash_profile_file" "$user_home/.bash_profile"; then
    printf "%s\n" "is same: .bash_profile for user: '$username'"

    is_run_push_script=false

  elif diff -q "$expected_bash_profile_file" <(head -n $(($(wc -l < "$expected_bash_profile_file") - 1)) "$user_home/.bash_profile"); then
    printf "%s\n" "is prefix: .bash_profile for user: '$username'"

    is_run_push_script=false
  else
    is_error=true

    printf "%s\n" "is different: .bash_profile for user: '$username'"
  fi
else
  printf "%s\n" "is missing: .bash_profile for user: '$username'"
fi
printf "%s\n" "checking: .bashrc for user: '$username'"

if [ -f "$user_home/.bashrc" ]; then
  printf "%s\n" "$combined_bashrc_content" > "$expected_bashrc_file"

  if diff -q "$expected_bashrc_file" "$user_home/.bashrc"; then
    printf "%s\n" "is same: .bashrc for user: '$username'"

    is_run_push_script=false

  elif diff -q "$expected_bashrc_file" <(head -n $(($(wc -l < "$expected_bashrc_file") - 1)) "$user_home/.bashrc"); then
    printf "%s\n" "is prefix: .bashrc for user: '$username'"

    is_run_push_script=false
  else
    is_error=true

    printf "%s\n" "is different: .bashrc for user: '$username'"
  fi
else
  printf "%s\n" "is missing: .bashrc for user: '$username'"
fi

if \$is_error; then
  printf "%s\n" "is error: dotfile check failed for user: '$username'"

  exit 1
else
  printf "%s\n" "done: dotfile check for user: '$username'"

  if \$is_run_push_script; then
    bash "$push_script_file" || exit $?
  fi
fi
EOF
  printf "%s\n" "${check_script_content_lines[@]}" > "$check_script_file"

  # Check if --force is not set

  if $is_force; then
    bash "$push_script_file" || return $?
  else
    SAVED_CHECK_SHELL_OPTIONS=$(set +o)

    set +e

    bash "$check_script_file"

    check_exit_code=$?

    eval "$SAVED_CHECK_SHELL_OPTIONS"

    if [ "$check_exit_code" -ne 0 ]; then
      return "$check_exit_code"
    fi
  fi
}

# If script needs root permissions: check if it's running as root
if [ "$EUID" -ne 0 ]; then
  printf "%s\n" "current user: $USER (not root)"

  # Check if any of the arguments is a user other than the current user
  is_needs_root=false

  for arg in "${@}"; do

    if [ "$arg" != "$USER" ] && [ "${arg:0:1}" != "-" ]; then
      is_needs_root=true

      printf "%s\n" "script needs: root"

      break
    fi
  done

  if $is_needs_root; then
    printf "%s\n" "rerunning script using: sudo"

    sudo "${0}" "${@}" # Rerun script with the same arguments using sudo

    printf "%s\n" "sudo exit code: $?"

    exit $?        # Exit with the status code of the sudo command
  else
    printf "%s\n" "continuing script without: sudo"
  fi
else
  printf "%s\n" "current user: root"
  printf "%s\n" "continuing script as: root"
fi

is_captured_args=false
is_force=false
args=()

if [ "${#}" -gt 0 ]; then
  printf "%s %s\n" "info: raw arguments:" "${*}"

  for arg in "${@}"; do

    if [ "${arg:0:1}" == "-" ]; then
      is_captured_args=true

      if [ "$arg" == "--force" ]; then
        is_force=true
      else
        printf "%s\n" "error: unknown flag argument: $arg"

        exit 1
      fi
    else
      args+=("$arg")
    fi
  done

  if $is_captured_args; then
    printf "%s\n" "info: flag:: force: $is_force"

    if [ "${#args[@]}" -gt 0 ]; then
      printf "%s %s\n" "info: uncaptured arguments:" "${args[*]}"
    fi
  fi
fi

users=()

if [ "${#args[@]}" -eq 0 ]; then
  users=("$USER")

  printf "%s %s\n" "info: no user provided: default to current user:" "${users[*]}"
else
  users=("${args[@]}")
fi

printf "%s\n" "start: processing users"

if [ "${#users[@]}" -eq 0 ]; then
  printf "%s\n" "info: no user to process"

  exit 0
else
  printf "%s %s\n" "info: users to process:" "${users[*]}"
fi

iter process_user {} "$is_force" "$bash_profile_global_content" "$bashrc_global_content" ::: "${users[@]}"
exit_code=$?

if [ $exit_code -ne 0 ]; then
  printf "%s\n" "is error: processing users failed"
  printf "%s\n" "exit code: $exit_code"

  exit $exit_code
else
  printf "%s\n" "done: processing users"
  printf "%s\n" "exit code: $exit_code"
fi

printf "%s\n" "done: setup-bash-for-user script"

exit 0
