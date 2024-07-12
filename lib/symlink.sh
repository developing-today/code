#!/usr/bin/env bash

printf "%s\n" "start: symlink script"

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
set -euo pipefail

source ./export-lib.sh

# TODO: USE LOCAL VARIABLES
# shellcheck disable=SC2317
create_symlink() {
  printf "create_symlink: args:: %s %s\n" "${#}" "${*}"

  is_relative_symlink="${1}"
  file_path="${2}"
  target_directory="${3}"

  printf "start: symlink for: relative symlink: '%s', file: '%s', directory: '%s'.\n" "${is_relative_symlink}" "${file_path}" "${target_directory}"

  # Check if the target directory is valid

  if [ ! -d "${target_directory}" ]; then
    printf "error: %s is not a directory. file_path: '%s', directory: '%s', is_relative_symlink: '%s'\n" "${target_directory}" "${is_relative_symlink}" "${file_path}" "${target_directory}"

    return 1
  fi

  # Get the file name from the file path
  symlink_name="${target_directory}/$(basename "${file_path}")"

  if [ -e "${symlink_name}" ]; then
    printf "skip: file or directory '%s' already exists.\n" "${symlink_name}"
  else
    # Check if the file exists
    if [ ! -f "${file_path}" ]; then
      printf "error: '%s' does not exist or is not a regular file.\n" "${file_path}"
      return 1
    fi

    # Determine the path to the file (relative or absolute)

    symlink_path=""

    if [ "${is_relative_symlink}" = true ]; then
      printf "start: generate relative symlink path.\n"

      symlink_path=$(realpath --relative-to="${target_directory}" "${file_path}")
    else
      printf "start: generate absolute symlink path.\n"

      symlink_path=$(realpath "${file_path}")
    fi
    printf "done: generate symlink path: %s, file_path: %s, directory: %s\n" "${symlink_path}" "${file_path}" "${target_directory}"

    if [ -z "${symlink_path}" ]; then
      printf "error: symlink path is empty for file_path '%s', target_directory '%s', is_relative_symlink '%s'.\n" "${is_relative_symlink}" "${file_path}" "${target_directory}"

      return 1
    fi

    printf "start: symlink at source '%s' to target '%s'.\n" "${symlink_path}" "${symlink_name}"

    ln -s "${symlink_path}" "${symlink_name}"

    printf "done: symlink created at source '%s' to target '%s'.\n" "${symlink_path}" "${symlink_name}"
  fi
  printf "done: created symlink for: relative symlink: '%s', file '%s', directory '%s'.\n" "${is_relative_symlink}" "${file_path}" "${target_directory}"
}

# TODO: USE LOCAL VARIABLES
# shellcheck disable=SC2317
process_symlinks() {
  printf "process_symlinks: args:: %s %s\n" "${#}" "${*}"

  is_relative_symlink="${1}"
  file_path=$(realpath "${2}")
  shift 2

  printf "start: symlinks directories list for file '%s', relative symlink '%s', target directories %s '%s'\n" "${file_path}" "${is_relative_symlink}" "${#}" "${*}"

  find_output=$(find "${@}" -maxdepth 1 -type d)

  printf "info: find output:\n%s\n" "${find_output}"

  mapfile -t directories <<< "${find_output}"

  printf "done: symlinks directory list generated for file '%s', relative symlink '%s', directories '%s'\n" "${file_path}" "${is_relative_symlink}" "${directories[*]}"
  printf "start: symlinks creation process for file '%s', relative symlink '%s', directories '%s'\n" "${file_path}" "${is_relative_symlink}" "${directories[*]}"

  # Using iter_3at3 to iterate through the directories
  iter create_symlink "${is_relative_symlink}" "${file_path}" {} ::: "${directories[@]}"

  return_code=$?

  printf "done: symlinks creation process for file '%s', relative symlink '%s', directories '%s', return code: %s\n" "${file_path}" "${is_relative_symlink}" "${directories[*]}" "${return_code}"

  return "${return_code}"
}

# Capture args
is_captured_args=false
is_relative_symlink=true
args=()

if [ "${#}" -gt 0 ]; then
  printf "%s %s\n" "info: raw arguments: " "${*}"

  for arg in "${@}"; do

    if [ "${arg:0:1}" == "-" ]; then
      is_captured_args=true

      if [ "${arg}" == "--absolute" ]; then
        is_relative_symlink=false

      elif [ "${arg}" == "--relative" ]; then
        is_relative_symlink=true
      else
        printf "error: unknown flag argument: %s\n" "${arg}"

        exit 1
      fi
    else
      args+=("${arg}")
    fi
  done

  if ${is_captured_args}; then

    if [ "${#args[@]}" -gt 0 ]; then
      printf "%s %s\n" "info: uncaptured arguments: " "${args[*]}"
    fi
  fi
fi

# Check the number of arguments

if [ "${#args[@]}" -eq 0 ] || [ "${#args[@]}" -eq 1 ]; then
  printf "error: Please provide both file_path and target_directory.\n"
  exit 1
else
  printf "info: file_path: %s, is_relative_symlink: %s, directories: %s\n" "${args[0]}" "${is_relative_symlink}" "${args[*]:1}"

  process_symlinks "${is_relative_symlink}" "${args[0]}" "${args[@]:1}"

  return_code=$?

  if [ "${return_code}" -ne 0 ]; then
    printf "%s\n" "is error: processing directories failed"
    printf "%s\n" "exit: $return_code"

    exit "${return_code}"
  else
    printf "%s\n" "done: processing directories"
    printf "%s\n" "exit: $return_code"
  fi
fi

printf "%s\n" "done: symlink script"

exit 0
