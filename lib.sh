#!/usr/bin/env bash

printf "%s\n" "start: lib script"

# # START restore shell options
# # copy into source script, uncomment SAVE_SHELL_OPTIONS, trap, and set
# # do not use trap in lib script, try not to avoid
# SAVED_SHELL_OPTIONS=$(set +o)

# shellcheck disable=SC2317,SC2154
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
  local cmd arg_1 arg_2 args exit_code
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
  local cmd arg_1 arg_2 args exit_code
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
  local cmd arg_1 arg_2 arg_3 args exit_code
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
  local cmd arg_1 arg_2 arg_3 args exit_code
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
  local cmd arg_1 arg_2 arg_3 arg_4 args exit_code
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
    # local ARCH # purposefully global, use carefully
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
    # local OS # purposefully global, use carefully
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
  local DETECTED_PROFILE SHELLTYPE
  DETECTED_PROFILE=''
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
  local PROFILE_FILE
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
  local target_dir
  target_dir=${1:-.}
  find "$target_dir" -type f \( -name "*.sh" -o -name "*.ps1" -o -name "*.py" \) -exec chmod +x {} +
}

recursively_remove_empty_folders_modified_more_than_14_days_ago() {
  local target_dir
  target_dir=${1:-.}
  find "$target_dir" -type d -empty -mtime +14 -execdir rmdir --ignore-fail-on-non-empty {} + 2>/dev/null
}

recursively_remove_zero_byte_files_modified_more_than_14_days_ago() {
  local target_dir
  target_dir=${1:-.}
  find "$target_dir" -type f -empty -mtime +14 -exec rm -f {} +
}

git_repo_root() {
  git rev-parse --show-toplevel
}

relative() {
  local target_location base_location relative_path

  target_location="$1"
  base_location="$2"
  relative_path=$(realpath --relative-to="$target_location" "$base_location")

  echo "$relative_path"
}

relative_git_repo_root() {
  local target_location repo_root relative_path

  target_location="${1:-.}"
  repo_root=$(git_repo_root)
  relative_path=$(relative "$target_location" "$repo_root")

  echo "$relative_path"
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

git_restore_all_and_clean() {
  git_restore_all
  git clean -fd
}

git_restore_all_and_clean_and_reset() {
  git_restore_all_and_clean
  git reset --hard
}

git_restore_all_and_clean_and_reset_and_pull() {
  git_restore_all_and_clean_and_reset
  git pull
}

git_restore_all_and_clean_and_reset_and_pull_and_prune() {
  git_restore_all_and_clean_and_reset_and_pull
  git remote prune origin
}

git_restore_all_and_clean_and_reset_and_pull_and_prune_and_gc() {
  git_restore_all_and_clean_and_reset_and_pull_and_prune
  git gc --prune=now
}

random_emoji() {
  # Emoji ranges
  local ranges=(
    "0x1F600 0x1F64F" # Emoticons
    "0x1F300 0x1F5FF" # Misc Symbols and Pictographs
    "0x1F700 0x1F77F" # Alchemical Symbols
    "0x1F800 0x1F8FF" # Supplemental Arrows-C
    "0x2600 0x26FF"   # Misc Symbols
    "0x2700 0x27BF"   # Dingbats
    "0x2300 0x23FF"   # Misc Technical
    # 2023-08-05 https://util.unicode.org/UnicodeJsps/list-unicodeset.jsp?a=%5B%3AEmoji%3DYes%3A%5D&abb=on&c=on&esc=on&g=&i=
    "0x23 0x2A 0x30 0x39" "0xA9" "0xAE" "0x203C" "0x2049" "0x2122" "0x2139" "0x2194 0x2199" "0x21A9 0x21AA" "0x231A 0x231B" "0x2328"
    "0x23CF" "0x23E9 0x23F3" "0x23F8 0x23FA" "0x24C2" "0x25AA 0x25AB" "0x25B6" "0x25C0" "0x25FB 0x25FE" "0x2600 0x2604" "0x260E" "0x2611"
    "0x2614 0x2615" "0x2618" "0x261D" "0x2620" "0x2622 0x2623" "0x2626" "0x262A" "0x262E 0x262F" "0x2638 0x263A" "0x2640" "0x2642" "0x2648 0x2653"
    "0x265F" "0x2660" "0x2663" "0x2665 0x2666" "0x2668" "0x267B" "0x267E" "0x267F" "0x2692 0x2697" "0x2699" "0x269B 0x269C" "0x26A0" "0x26A1"
    "0x26A7" "0x26AA 0x26AB" "0x26B0 0x26B1" "0x26BD 0x26BE" "0x26C4 0x26C5" "0x26C8" "0x26CE" "0x26CF" "0x26D1" "0x26D3 0x26D4" "0x26E9 0x26EA"
    "0x26F0 0x26F5" "0x26F7 0x26FA" "0x26FD" "0x2702" "0x2705" "0x2708 0x270D" "0x270F" "0x2712" "0x2714" "0x2716" "0x271D" "0x2721" "0x2728"
    "0x2733 0x2734" "0x2744" "0x2747" "0x274C" "0x274E" "0x2753 0x2755" "0x2757" "0x2763 0x2764" "0x2795 0x2797" "0x27A1" "0x27B0" "0x27BF"
    "0x2934 0x2935" "0x2B05 0x2B07" "0x2B1B 0x2B1C" "0x2B50" "0x2B55" "0x3030" "0x303D" "0x3297" "0x3299"
    "0x1F004" "0x1F0CF" "0x1F170 0x1F171" "0x1F17E 0x1F17F" "0x1F18E" "0x1F191 0x1F19A" "0x1F1E6 0x1F1FF" "0x1F201 0x1F202" "0x1F21A"
    "0x1F22F" "0x1F232 0x1F23A" "0x1F250 0x1F251" "0x1F300 0x1F321" "0x1F324 0x1F393" "0x1F396 0x1F397" "0x1F399 0x1F39B" "0x1F39E 0x1F3F0"
    "0x1F3F3 0x1F3F5" "0x1F3F7 0x1F4FD" "0x1F4FF 0x1F53D" "0x1F549 0x1F54E" "0x1F550 0x1F567" "0x1F56F" "0x1F570" "0x1F573 0x1F57A"
    "0x1F587" "0x1F58A 0x1F58D" "0x1F590" "0x1F595 0x1F596" "0x1F5A4" "0x1F5A5" "0x1F5A8" "0x1F5B1 0x1F5B2" "0x1F5BC" "0x1F5C2 0x1F5C4"
    "0x1F5D1 0x1F5D3" "0x1F5DC 0x1F5DE" "0x1F5E1" "0x1F5E3" "0x1F5E8" "0x1F5EF" "0x1F5F3" "0x1F5FA 0x1F64F" "0x1F680 0x1F6C5" "0x1F6CB 0x1F6D2"
    "0x1F6D5 0x1F6D7" "0x1F6DC 0x1F6E5" "0x1F6E9" "0x1F6EB 0x1F6EC" "0x1F6F0" "0x1F6F3 0x1F6FC" "0x1F7E0 0x1F7EB" "0x1F7F0" "0x1F90C 0x1F93A"
    "0x1F93C 0x1F945" "0x1F947 0x1F9FF" "0x1FA70 0x1FA7C" "0x1FA80 0x1FA88" "0x1FA90 0x1FABD" "0x1FABF 0x1FAC5" "0x1FACE 0x1FADB"
    "0x1FAE0 0x1FAE8" "0x1FAF0 0x1FAF8"
  )

  # Choose a random range
  local range=${ranges[$RANDOM % ${#ranges[@]}]}
  local start=$(echo $range | awk '{ print $1 }')
  local end=$(echo $range | awk '{ print $2 }')

  # Generate a random number within the chosen range
  local random_number=$((RANDOM % (end - start + 1) + start))

  # Convert the random number to the corresponding Unicode character
  printf "\\U$(printf '%x' $random_number)"
}

random_word() {
  local words=(
    # Fruits
    "apple" "banana" "cherry" "date" "elderberry" "fig" "grape" "kiwi" "lemon" "mango" "peach" "pear" "pineapple" "plum" "pomegranate" "watermelon" "blueberry" "coconut" "apricot" "blackberry" "raspberry" "strawberry" "nectarine" "orange" "lime" "tangerine" "grapefruit" "cantaloupe" "honeydew" "durian" "lychee" "passionfruit"
    # Vegetables
    "carrot" "broccoli" "asparagus" "spinach" "pepper" "tomato" "onion" "cucumber" "lettuce" "kale" "radish" "celery" "squash" "zucchini" "beet" "parsnip" "cabbage" "cauliflower" "eggplant" "fennel" "garlic" "leek" "mushroom" "okra" "peas" "potato" "pumpkin" "rutabaga" "sweet_potato" "turnip" "yam" "artichoke"
    # Animals
    "elephant" "tiger" "bear" "zebra" "giraffe" "dolphin" "whale" "eagle" "panda" "wolf" "lion" "cheetah" "kangaroo" "hippo" "rhino" "flamingo" "alligator" "anteater" "armadillo" "baboon" "badger" "bat" "beaver" "buffalo" "camel" "chameleon" "chimpanzee" "cobra" "crocodile" "deer" "dingo" "fox" "gorilla"
    # Pokémon
    "Pikachu" "Charizard" "Bulbasaur" "Squirtle" "Jigglypuff" "Meowth" "Gengar" "Mewtwo" "Eevee" "Snorlax" "Lucario" "Gardevoir" "Greninja" "Mimikyu" "Rayquaza" "Sylveon" "Blastoise" "Venusaur" "Charmander" "Machamp" "Lapras" "Arcanine" "Mew" "Lugia" "Ho-Oh" "Kyogre" "Groudon" "Arceus" "Dialga" "Palkia" "Giratina" "Reshiram"
    # Mythical Beasts
    "dragon" "phoenix" "griffin" "sphinx" "minotaur" "unicorn" "kraken" "goblin" "harpy" "chimera" "wyvern" "siren" "nymph" "basilisk" "yeti" "mermaid" "cerberus" "banshee" "centaur" "chupacabra" "cyclops" "djinn" "doppelganger" "dryad" "elf" "fairy" "faun" "genie" "ghost" "gorgon" "gremlin" "imp"
    # Rocks
    "granite" "marble" "limestone" "basalt" "quartz" "slate" "obsidian" "amethyst" "sandstone" "shale" "jade" "opal" "dolomite" "gypsum" "pyrite" "sapphire" "agate" "alabaster" "andesite" "aquamarine" "beryl" "calcite" "chalk" "chert" "clay" "coal" "corundum" "diamond" "diorite" "dunite" "emerald" "flint"
    # Planets (including dwarf planets and exoplanets)
    "Mercury" "Venus" "Earth" "Mars" "Jupiter" "Saturn" "Uranus" "Neptune" "Pluto" "Ceres" "Haumea" "Makemake" "Eris" "Quaoar" "Sedna" "Orcus" "Gonggong" "Salacia" "Varuna" "Ixion" "Chaos" "Deedee" "Haumea" "Makemake" "Oberon" "Titania" "Ariel" "Umbriel" "Triton" "Proteus" "Charon" "Nix"
    # Astrological Bodies
    "Sun" "Moon" "Sirius" "Orion" "Pleiades" "Andromeda" "Vega" "Polaris" "Rigel" "Betelgeuse" "Altair" "Deneb" "Antares" "Canopus" "Aldebaran" "Spica" "Fomalhaut" "Regulus" "Pollux" "Capella" "Bellatrix" "Castor" "Diphda" "Elnath" "Gacrux" "Hamal" "Kaus_Australis" "Menkar" "Mirfak" "Naos" "Saiph" "Shaula"
    # Types of Astrological Objects
    "black_hole" "comet" "nebula" "galaxy" "asteroid" "pulsar" "quasar" "meteor" "white_dwarf" "satellite" "Hubble_Space_Telescope" "International_Space_Station" "Elon's_Tesla" "brown_dwarf" "gamma_ray_burst" "magnetar" "nova" "rogue_planet" "shooting_star" "solar_flare" "space_probe" "space_shuttle" "star_cluster" "supernova" "telescope" "wormhole" "x-ray_binary"
    # Common Dog Names
    "Bella" "Max" "Lucy" "Charlie" "Cooper" "Buddy" "Molly" "Daisy" "Bailey" "Sadie" "Rocky" "Rosie" "Chloe" "Coco" "Zeus" "Lola" "Duke" "Bear" "Oliver" "Winston" "Lily" "Zoe" "Riley" "Abby" "Ginger" "Roxy" "Ruby" "Sasha" "Stella" "Tucker" "Bentley" "Jackson" "Lady" "Lulu"
    # Common Cat Names
    "Luna" "Oliver" "Bella" "Chloe" "Leo" "Milo" "Charlie" "Max" "Simba" "Lily" "Smokey" "Shadow" "Tiger" "Nala" "Felix" "Whiskers" "Cleo" "Garfield" "Jasper" "Kitty" "Mittens" "Oscar" "Paws" "Princess" "Pumpkin" "Sassy" "Simba" "Snowball" "Sophie" "Sparky" "Tigger" "Tom" "Ziggy"
    # Characters from Final Fantasy
    "Cloud" "Tifa" "Aerith" "Sephiroth" "Squall" "Rinoa" "Zidane" "Yuna" "Noctis" "Lightning" "Cecil" "Rydia" "Kain" "Bartz" "Terra" "Locke" "Barret" "Vivi" "Auron" "Fran" "Basch" "Serah" "Hope" "Zack" "Vincent" "Rikku" "Selphie" "Seifer" "Garnet" "Edgar" "Sabin" "Setzer"
    # Characters from Fire Emblem
    "Marth" "Ike" "Roy" "Lucina" "Chrom" "Robin" "Corrin" "Byleth" "Edelgard" "Dimitri" "Sigurd" "Eliwood" "Lyn" "Micaiah" "Tharja" "Camilla" "Alm" "Celica" "Eirika" "Ephraim" "Hector" "Leif" "Ninian" "Olwen" "Reinhardt" "Seliph" "Sothe" "Takumi" "Tiki" "Xander" "Azura" "Fjorm"
  )
  printf "%s" "${words[$RANDOM % ${#words[@]}]}"
}

random_emoji_name() {
  local emoji_name
  # Check for required commands
  if ! command -v jq &>/dev/null || ! command -v shuf &>/dev/null; then
    echo "Required commands jq or shuf not found." >&2
    emoji_name="" # Ensure emoji_name is empty if an error occurred
  else
    emoji_name=$(curl -s https://api.github.com/emojis 2>/dev/null | jq -r 'keys[]' | shuf -n 1 || echo "" || true)
  fi

  # If the emoji_name is empty, use a safer alternative from a predefined list
  if [[ -z "${emoji_name}" || "${emoji_name}" == "true" || "${emoji_name}" == "false" || "${emoji_name}" == "null" || "${emoji_name}" == "undefined" || "${emoji_name}" == "error" || "${emoji_name}" == "not_found" || "${emoji_name}" == "rate_limit_exceeded" || "${emoji_name}" == "invalid_credentials" || "${emoji_name}" == "api_usage" || "${emoji_name}" == "abuse_detected" || "${emoji_name}" == "file_too_large" || "${emoji_name}" == "unsupported_media_type" || "${emoji_name}" == "unprocessable" || "${emoji_name}" == "server_error" || "${emoji_name}" == "temporarily_unavailable" ]]; then
    emoji_name=$(random_word)
  fi

  printf "%s" "${emoji_name}"

  return 0 # Explicitly return 0 to ensure the function never exits with an error
}

datetime() {
  printf "%s" "$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
}

# shellcheck disable=SC2059,SC2317
git_commit_all() {
  local commit_message random_emojis random_emoji_name
  random_emojis="$(random_emoji) $(random_emoji) $(random_emoji)"
  random_emoji_name=$(random_emoji_name || random_word)
  commit_message=${1:-"${random_emojis}  ${random_emoji_name} $(datetime)"}

  git add -A
  git commit -m "${commit_message}"
}

printf "%s\n" "done: lib script"
