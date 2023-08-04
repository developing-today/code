#!/usr/bin/env bash

printf "%s\n" "start: lib script"

# # START restore shell options
# # copy into source script, uncomment SAVE_SHELL_OPTIONS, trap, and set
# # do not use trap in lib script, try not to avoid
# SAVED_SHELL_OPTIONS=$(set +o)

# shellcheck disable=SC2317
restore_shell_options() {
  printf "%s\n" "trap start: restoring shell options"
  # printf "%s\n" "SAVED_SHELL_OPTIONS: ${SAVED_SHELL_OPTIONS}"
  # printf "%s\n" "CURRENT_SHELL_OPTIONS: $(set +o)"

  eval "${SAVED_SHELL_OPTIONS}"

  printf "%s\n" "trap done: restoring shell options"
}
# trap restore_shell_options EXIT
# set -e
# # DONE restore shell option

# shellcheck disable=SC2317
iter_3at2() {
  printf "iter_3at2: args:: %s %s\n" "${#}" "${*}"

  cmd="${1}"
  arg_1="${2}"
  arg_2="${3}"

  shift 3

  args=("${@}")
  exit_code=0

  printf "iter_3at2 start:: cmd: %s, arg_1: %s, arg_2: %s, args: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${args[*]}"

  if command -v parallel >/dev/null 2>&1; then
    printf "iter_3at2 parallel start:: cmd: %s, arg_1: %s, arg_2: %s, args: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${args[*]}"

    parallel "${cmd}" "${arg_1}" {} "${arg_2}" ::: "${args[@]}"

    exit_code=$?

    printf "iter_3at2 parallel done:: cmd: %s, arg_1: %s, arg_2: %s, args: %s, exit code: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${args[*]}" "${exit_code}"

  else
    printf "iter_3at2 sequential start:: cmd: %s, arg_1: %s, arg_2: %s, args: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${args[*]}"

    for arg in "${args[@]}"; do
      printf "iter_3at2 sequential start arg:: cmd: %s, arg_1: %s, arg: %s, arg_2: %s\n" "${cmd}" "${arg_1}" "${arg}" "${arg_2}"

      $cmd "${arg_1}" "${arg}" "${arg_2}"
      for_exit_code=$?

      printf "iter_3at2 sequential done arg:: cmd: %s, arg_1: %s, arg: %s, arg_2: %s, exit code: %s\n" "${cmd}" "${arg_1}" "${arg}" "${arg_2}" "${for_exit_code}"

      if [ "${for_exit_code}" -ne 0 ]; then
        exit_code=$for_exit_code
      fi
    done
  fi

  printf "iter_3at2 done:: cmd: %s, arg_1: %s, arg_2: %s, args: %s, exit code: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${args[*]}" "${exit_code}"

  return "${exit_code}"
}

# shellcheck disable=SC2317

iter_3at3() {
  printf "iter_3at3: args:: %s %s\n" "${#}" "${*}"

  cmd="${1}"
  arg_1="${2}"
  arg_2="${3}"

  shift 3

  args=("${@}")
  exit_code=0

  printf "iter_3at3 start:: cmd: %s, arg_1: %s, arg_2: %s, args:: %s %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${#args[@]}" "${args[*]}"

  if command -v parallel >/dev/null 2>&1; then
    printf "iter_3at3 parallel start:: cmd: %s, arg_1: %s, arg_2: %s, args:: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${args[*]}"

    parallel "${cmd}" "${arg_1}" "${arg_2}" {} ::: "${args[@]}"

    exit_code=$?

    printf "iter_3at3 parallel done:: cmd: %s, arg_1: %s, arg_2: %s, args:: %s, exit code: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${args[*]}" "${exit_code}"

  else
    printf "iter_3at3 sequential start:: cmd: %s, arg_1: %s, arg_2: %s, args:: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${args[*]}"

    for arg in "${args[@]}"; do
      printf "iter_3at3 sequential start arg:: cmd: %s, arg_1: %s, arg_2: %s, arg: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${arg}"

      $cmd "${arg_1}" "${arg_2}" "${arg}"
      for_exit_code=$?

      printf "iter_3at3 sequential done arg:: cmd: %s, arg_1: %s, arg_2: %s, arg: %s, exit code: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${arg}" "${for_exit_code}"

      if [ "${for_exit_code}" -ne 0 ]; then
        exit_code=$for_exit_code
      fi
    done
  fi

  printf "iter_3at3 done:: cmd: %s, arg_1: %s, arg_2: %s, args:: %s, exit code: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${args[*]}" "${exit_code}"

  return "${exit_code}"
}

# shellcheck disable=SC2317
iter_4at1() {
  printf "iter_4at1: args:: %s %s\n" "${#}" "${*}"

  cmd="${1}"
  arg_1="${2}"
  arg_2="${3}" # customized: skipped for size in printf
  arg_3="${4}" # customized: skipped for size in printf

  shift 4

  args=("${@}")
  exit_code=0

  printf "iter_4at1 start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s\n" "${cmd}" "${arg_1}" "\${arg_2} skipped for size" "\${arg_3} skipped for size" "${args[*]}"

  if command -v parallel >/dev/null 2>&1; then
    printf "iter_4at1 parallel start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s\n" "${cmd}" "${arg_1}" "\${arg_2} skipped for size" "\${arg_3} skipped for size" "${args[*]}"

    parallel "${cmd}" {} "${arg_1}" "${arg_2}" "${arg_3}" ::: "${args[@]}"

    exit_code=$?

    printf "iter_4at1 parallel done:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s, exit code: %s\n" "${cmd}" "${arg_1}" "\${arg_2} skipped for size" "\${arg_3} skipped for size" "${args[*]}" "${exit_code}"
  else
    printf "iter_4at1 sequential start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s\n" "${cmd}" "${arg_1}" "\${arg_2} skipped for size" "\${arg_3} skipped for size" "${args[*]}"

    for arg in "${args[@]}"; do
      printf "iter_4at1 sequential start arg:: cmd: %s, arg: %s, arg_1: %s, arg_2: %s, arg_3: %s\n" "${cmd}" "${arg}" "${arg_1}" "\${arg_2} skipped for size" "\${arg_3} skipped for size"

      $cmd "${arg}" "${arg_1}" "${arg_2}" "${arg_3}"
      for_exit_code=$?

      printf "iter_4at1 sequential done arg:: cmd: %s, arg: %s, arg_1: %s, arg_2: %s, arg_3: %s, exit code: %s\n" "${cmd}" "${arg}" "${arg_1}" "\${arg_2} skipped for size" "\${arg_3} skipped for size" "${for_exit_code}"

      if [ "${for_exit_code}" -ne 0 ]; then
        exit_code=$for_exit_code
      fi
    done
  fi

  printf "iter_4at1 done:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s, exit code: %s\n" "${cmd}" "${arg_1}" "\${arg_2} skipped for size" "\${arg_3} skipped for size" "${args[*]}" "${exit_code}"

  return "${exit_code}"
}

# shellcheck disable=SC2317
iter_4at3() {
  printf "iter_4at3: args:: %s %s\n" "${#}" "${*}"

  cmd="${1}"
  arg_1="${2}"
  arg_2="${3}"
  arg_3="${4}"

  shift 4

  args=("${@}")
  exit_code=0

  printf "iter_4at3 start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${arg_3}" "${args[*]}"

  if command -v parallel >/dev/null 2>&1; then
    printf "iter_4at3 parallel start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${arg_3}" "${args[*]}"

    parallel "${cmd}" "${arg_1}" "${arg_2}" {} "${arg_3}" ::: "${args[@]}"

    exit_code=$?

    printf "iter_4at3 parallel done:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s, exit code: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${arg_3}" "${args[*]}" "${exit_code}"

  else
    printf "iter_4at3 sequential start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${arg_3}" "${args[*]}"

    for arg in "${args[@]}"; do
      printf "iter_4at3 sequential start arg:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${arg_3}" "${arg}"

      $cmd "${arg_1}" "${arg_2}" "${arg}" "${arg_3}"

      for_exit_code=$?

      printf "iter_4at3 sequential done arg:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg: %s, exit code: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${arg_3}" "${arg}" "${for_exit_code}"

      if [ $for_exit_code -ne 0 ]; then
        exit_code=$for_exit_code
      fi
    done
  fi

  printf "iter_4at3 done:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s, exit code: %s\n" "${cmd}" "${arg_1}" "${arg_2}" "${arg_3}" "${args[*]}" "${exit_code}"

  return $exit_code
}

# shellcheck disable=SC2317
iter_5at1() {
  printf "iter_5at1: args:: %s %s\n" "${#}" "${*}"

  cmd="${1}"
  arg_1="${2}"
  arg_2="${3}" # customized: skipped for size in printf
  arg_3="${4}" # customized: skipped for size in printf
  arg_4="${5}"

  shift 5

  args=("${@}")
  exit_code=0

  printf "iter_5at1 start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg_4: %s, args: %s\n" "${cmd}" "${arg_1}" "\${arg_2} skipped for size" "\${arg_3} skipped for size" "${arg_4}" "${args[*]}"

  if command -v parallel >/dev/null 2>&1; then
    printf "iter_5at1 parallel start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg_4: %s, args: %s\n" "${cmd}" "${arg_1}" "\${arg_2} skipped for size" "\${arg_3} skipped for size" "${arg_4}" "${args[*]}"

    parallel "${cmd}" {} "${arg_1}" "${arg_2}" "${arg_3}" "${arg_4}" ::: "${args[@]}"

    exit_code=$?

    printf "iter_5at1 parallel done:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg_4: %s, args: %s, exit code: %s\n" "${cmd}" "${arg_1}" "\${arg_2} skipped for size" "\${arg_3} skipped for size" "${arg_4}" "${args[*]}" "${exit_code}"
  else
    printf "iter_5at1 sequential start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg_4: %s, args: %s\n" "${cmd}" "${arg_1}" "\${arg_2} skipped for size" "\${arg_3} skipped for size" "${arg_4}" "${args[*]}"

    for arg in "${args[@]}"; do
      printf "iter_5at1 sequential start arg:: cmd: %s, arg: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg_4: %s\n" "${cmd}" "${arg}" "${arg_1}" "\${arg_2} skipped for size" "\${arg_3} skipped for size" "${arg_4}"

      $cmd "${arg}" "${arg_1}" "${arg_2}" "${arg_3}" "${arg_4}"
      for_exit_code=$?

      printf "iter_5at1 sequential done arg:: cmd: %s, arg: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg_4: %s, exit code: %s\n" "${cmd}" "${arg}" "${arg_1}" "\${arg_2} skipped for size" "\${arg_3} skipped for size" "${arg_4}" "${for_exit_code}"

      if [ "${for_exit_code}" -ne 0 ]; then
        exit_code=$for_exit_code
      fi
    done
  fi

  printf "iter_5at1 done:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg_4: %s, args: %s, exit code: %s\n" "${cmd}" "${arg_1}" "\${arg_2} skipped for size" "\${arg_3} skipped for size" "${arg_4}" "${args[*]}" "${exit_code}"

  return "${exit_code}"
}

reset="\033[0m"
bright_blue="${reset}\033[34;1m"

# shellcheck disable=SC2059
probe_arch() {
    ARCH=$(uname -m)
    case $ARCH in
        x86_64) ARCH="x86_64"  ;;
        aarch64) ARCH="arm64" ;;
        arm64) ARCH="arm64" ;;
        *) printf "Architecture ${ARCH} is not supported by this installation script\n"; exit 1 ;;
    esac
}

# shellcheck disable=SC2059
probe_os() {
    OS=$(uname -s)
    case $OS in
        Darwin) OS="Darwin" ;;
        Linux) OS="Linux" ;;
        *) printf "Operating system ${OS} is not supported by this installation script\n"; exit 1 ;;
    esac
}

# shellcheck disable=SC2059,SC2140
print_logo() { # todo: make a logo
    printf "${bright_blue}
                 .:                                 .:
  .\$\$.   \$\$:   .\$\$\$:                                \$\$\$^    \$\$:   ~\$^
  .\$\$\$!:\$\$\$  .\$\$\$\$~                                 .\$\$\$\$^  !\$\$~^\$\$\$~
    \$\$\$\$\$\$ .\$\$\$\$\$~                                   .\$\$\$\$\$^ \$\$\$\$\$\$:
     !\$\$\$\$\$\$\$\$\$\$~                                     .\$\$\$\$\$\$\$\$\$\$\$
      :\$\$\$\$\$\$\$\$~                                       .\$\$\$\$\$\$\$\$!
     .\$\$\$\$\$\$\$\$~                                         .\$\$\$\$\$\$\$\$^
    .\$\$\$\$\$\$\$\$!       ~\$!                       :\$\$.      :\$\$\$\$\$\$\$\$^
     \$\$\$\$\$\$\$\$\$\$\$!^::\$\$\$\$\$^...................:\$\$\$\$\$!.^~\$\$\$\$\$\$\$\$\$\$\$:
     \$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$
     :\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$!
        :^!\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$~:.
           :\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$!
      \$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$:
      :\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$~
        ^\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$~.
           :\$\$\$\$\$:   .^~!\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$!^:.   \$\$\$\$\$!
           :\$\$\$\$\$!.         .!\$\$\$\$\$\$\$\$\$\$\$\$.         .^\$\$\$\$\$!
           :\$\$\$\$\$\$\$\$\$\$!^:.   ~\$\$\$\$\$\$\$\$\$\$\$\$    .^~\$\$\$\$\$\$\$\$\$\$!
           :\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$. ~\$\$\$\$\$\$\$\$\$\$\$\$  \$\$\$\$\$\$\$\$\$\$\$\$\$\$\$!
           :\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$: ~\$\$\$\$\$\$\$\$\$\$\$\$  \$\$\$\$\$\$\$\$\$\$\$\$\$\$\$!
           :\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$^ ~\$\$\$\$\$\$\$\$\$\$\$\$  \$\$\$\$\$\$\$\$\$\$\$\$\$\$\$!
           :\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$~ ~\$\$\$\$\$\$\$\$\$\$\$\$  \$\$\$\$\$\$\$\$\$\$\$\$\$\$\$!
           :\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$~^^:.     ..:^~!\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$!
           ^\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$!
           :\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$~
            :\$\$\$\$\$\$\$\$\$\$\$\$\$:\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$~~\$\$\$\$\$\$\$\$\$\$\$\$~
              !\$\$\$\$\$\$\$\$\$\$. :\$\$..\$\$! :\$\$^ !\$!  ~\$\$\$\$\$\$\$\$\$\$.
               ^\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$!
                 \$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$:
                  ~\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$
                   "\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$\$"
                     \$\$\$\$\$~\$\$\$\$\$^\$\$\$\$\$~\$\$\$\$\$\$~\$\$\$\$:
                      \$\$^  .\$\$\$   \$\$\$:  ~\$\$^  .\$\$^
                      ..     :     :     :.     :
${reset}
"
}

# shellcheck disable=SC2236
detect_profile() {
  local DETECTED_PROFILE
  DETECTED_PROFILE=''
  local SHELLTYPE
  SHELLTYPE="$(basename "/$SHELL")"

  if [ "$SHELLTYPE" = "bash" ]; then
    if [ -f "$HOME/.bashrc" ]; then
      DETECTED_PROFILE="$HOME/.bashrc"
    elif [ -f "$HOME/.bash_profile" ]; then
      DETECTED_PROFILE="$HOME/.bash_profile"
    fi
  elif [ "$SHELLTYPE" = "zsh" ]; then
    DETECTED_PROFILE="$HOME/.zshrc"
  elif [ "$SHELLTYPE" = "fish" ]; then
    DETECTED_PROFILE="$HOME/.config/fish/conf.d/turso.fish"
  fi

  if [ -z "$DETECTED_PROFILE" ]; then
    if [ -f "$HOME/.profile" ]; then
      DETECTED_PROFILE="$HOME/.profile"
    elif [ -f "$HOME/.bashrc" ]; then
      DETECTED_PROFILE="$HOME/.bashrc"
    elif [ -f "$HOME/.bash_profile" ]; then
      DETECTED_PROFILE="$HOME/.bash_profile"
    elif [ -f "$HOME/.zshrc" ]; then
      DETECTED_PROFILE="$HOME/.zshrc"
    elif [ -d "$HOME/.config/fish" ]; then
      DETECTED_PROFILE="$HOME/.config/fish/conf.d/turso.fish"
    fi
  fi

  if [ ! -z "$DETECTED_PROFILE" ]; then
    echo "$DETECTED_PROFILE"
  fi
}

# shellcheck disable=SC2059
update_profile_for_turso() {
   PROFILE_FILE=$(detect_profile)
   if ! grep -q "\.turso" "$PROFILE_FILE"; then
      printf "\n${bright_blue}Updating profile ${reset}$PROFILE_FILE\n"

      printf "\n# Turso\nexport PATH=\"$INSTALL_DIRECTORY:\$PATH\"\n" >> "$PROFILE_FILE"

      printf "\nTurso will be available when you open a new terminal.\n"
      printf "If you want to make Turso available in this terminal, please run:\n"
      printf "\nsource $PROFILE_FILE\n"
   fi
}

recursively_chmod_executable_scripts() {
  find "$1" -type f \( -name "*.sh" -o -name "*.ps1" -o -name "*.py" \) -exec chmod +x {} +
}

recursively_remove_empty_folders_modified_more_than_14_days_ago() {
  find "$1" -type d -empty -mtime +14 -execdir rmdir --ignore-fail-on-non-empty {} + 2>/dev/null
}

recursively_remove_zero_byte_files_modified_more_than_14_days_ago() {
  find "$1" -type f -empty -mtime +14 -exec rm -f {} +
}

git_repo_root() {
  git rev-parse --show-toplevel
}

git_restore_all_deleted_files() {
  git ls-files -d | xargs git checkout --
}

git_restore_all_modified_files() {
  git ls-files -m | xargs git checkout --
}

git_restore_all_untracked_files() {
  git ls-files -o --exclude-standard | xargs git checkout --
}

git_restore_all() {
  git_restore_all_deleted_files
  git_restore_all_modified_files
  git_restore_all_untracked_files
}

printf "%s\n" "done: lib script"
