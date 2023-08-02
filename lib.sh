#!/usr/bin/env bash

# shellcheck disable=SC2317
iter_3at2() {
  cmd="$1"
  arg_1="$2"
  arg_2="$3"

  shift 3

  args=("$@")
  exit_code=0

  printf "iter_3at2 start:: cmd: %s, arg_1: %s, arg_2: %s, args: %s\n" "$cmd" "$arg_1" "$arg_2" "${args[*]}"

  if command -v parallel >/dev/null 2>&1; then
    printf "iter_3at2 parallel start:: cmd: %s, arg_1: %s, arg_2: %s, args: %s\n" "$cmd" "$arg_1" "$arg_2" "${args[*]}"

    parallel "$cmd" "$arg_1" {} "$arg_2" ::: "${args[@]}"

    exit_code=$?

    printf "iter_3at2 parallel done:: cmd: %s, arg_1: %s, arg_2: %s, args: %s, exit code: %s\n" "$cmd" "$arg_1" "$arg_2" "${args[*]}" "$exit_code"

  else
    printf "iter_3at2 sequential start:: cmd: %s, arg_1: %s, arg_2: %s, args: %s\n" "$cmd" "$arg_1" "$arg_2" "${args[*]}"

    for arg in "${args[@]}"; do
      printf "iter_3at2 sequential start arg:: cmd: %s, arg_1: %s, arg: %s, arg_2: %s\n" "$cmd" "$arg_1" "$arg" "$arg_2"

      $cmd "$arg_1" "$arg" "$arg_2"
      for_exit_code=$?

      printf "iter_3at2 sequential done arg:: cmd: %s, arg_1: %s, arg: %s, arg_2: %s, exit code: %s\n" "$cmd" "$arg_1" "$arg" "$arg_2" "$for_exit_code"

      if [ "$for_exit_code" -ne 0 ]; then
        exit_code=$for_exit_code
      fi
    done
  fi

  printf "iter_3at2 done:: cmd: %s, arg_1: %s, arg_2: %s, args: %s, exit code: %s\n" "$cmd" "$arg_1" "$arg_2" "${args[*]}" "$exit_code"

  return "$exit_code"
}

# shellcheck disable=SC2317
iter_4at1() {
  cmd="$1"
  arg_1="$2"
  arg_2="$3" # customized: skipped for size in printf
  arg_3="$4" # customized: skipped for size in printf

  shift 4

  args=("$@")
  exit_code=0

  printf "iter_4at1 start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s\n" "$cmd" "$arg_1" "\$arg_2 skipped for size" "\$arg_3 skipped for size" "${args[*]}"

  if command -v parallel >/dev/null 2>&1; then
    printf "iter_4at1 parallel start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s\n" "$cmd" "$arg_1" "\$arg_2 skipped for size" "\$arg_3 skipped for size" "${args[*]}"

    parallel "$cmd" {} "$arg_1" "$arg_2" "$arg_3" ::: "${args[@]}"

    exit_code=$?

    printf "iter_4at1 parallel done:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s, exit code: %s\n" "$cmd" "$arg_1" "\$arg_2 skipped for size" "\$arg_3 skipped for size" "${args[*]}" "$exit_code"
  else
    printf "iter_4at1 sequential start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s\n" "$cmd" "$arg_1" "\$arg_2 skipped for size" "\$arg_3 skipped for size" "${args[*]}"

    for arg in "${args[@]}"; do
      printf "iter_4at1 sequential start arg:: cmd: %s, arg: %s, arg_1: %s, arg_2: %s, arg_3: %s\n" "$cmd" "$arg" "$arg_1" "\$arg_2 skipped for size" "\$arg_3 skipped for size"

      $cmd "$arg" "$arg_1" "$arg_2" "$arg_3"
      for_exit_code=$?

      printf "iter_4at1 sequential done arg:: cmd: %s, arg: %s, arg_1: %s, arg_2: %s, arg_3: %s, exit code: %s\n" "$cmd" "$arg" "$arg_1" "\$arg_2 skipped for size" "\$arg_3 skipped for size" "$for_exit_code"

      if [ "$for_exit_code" -ne 0 ]; then
        exit_code=$for_exit_code
      fi
    done
  fi

  printf "iter_4at1 done:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, args: %s, exit code: %s\n" "$cmd" "$arg_1" "\$arg_2 skipped for size" "\$arg_3 skipped for size" "${args[*]}" "$exit_code"

  return "$exit_code"
}

# shellcheck disable=SC2317
iter_5at1() {
  cmd="$1"
  arg_1="$2"
  arg_2="$3" # customized: skipped for size in printf
  arg_3="$4" # customized: skipped for size in printf
  arg_4="$5"

  shift 5

  args=("$@")
  exit_code=0

  printf "iter_5at1 start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg_4: %s, args: %s\n" "$cmd" "$arg_1" "\$arg_2 skipped for size" "\$arg_3 skipped for size" "$arg_4" "${args[*]}"

  if command -v parallel >/dev/null 2>&1; then
    printf "iter_5at1 parallel start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg_4: %s, args: %s\n" "$cmd" "$arg_1" "\$arg_2 skipped for size" "\$arg_3 skipped for size" "$arg_4" "${args[*]}"

    parallel "$cmd" {} "$arg_1" "$arg_2" "$arg_3" "$arg_4" ::: "${args[@]}"

    exit_code=$?

    printf "iter_5at1 parallel done:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg_4: %s, args: %s, exit code: %s\n" "$cmd" "$arg_1" "\$arg_2 skipped for size" "\$arg_3 skipped for size" "$arg_4" "${args[*]}" "$exit_code"
  else
    printf "iter_5at1 sequential start:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg_4: %s, args: %s\n" "$cmd" "$arg_1" "\$arg_2 skipped for size" "\$arg_3 skipped for size" "$arg_4" "${args[*]}"

    for arg in "${args[@]}"; do
      printf "iter_5at1 sequential start arg:: cmd: %s, arg: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg_4: %s\n" "$cmd" "$arg" "$arg_1" "\$arg_2 skipped for size" "\$arg_3 skipped for size" "$arg_4"

      $cmd "$arg" "$arg_1" "$arg_2" "$arg_3" "$arg_4"
      for_exit_code=$?

      printf "iter_5at1 sequential done arg:: cmd: %s, arg: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg_4: %s, exit code: %s\n" "$cmd" "$arg" "$arg_1" "\$arg_2 skipped for size" "\$arg_3 skipped for size" "$arg_4" "$for_exit_code"

      if [ "$for_exit_code" -ne 0 ]; then
        exit_code=$for_exit_code
      fi
    done
  fi

  printf "iter_5at1 done:: cmd: %s, arg_1: %s, arg_2: %s, arg_3: %s, arg_4: %s, args: %s, exit code: %s\n" "$cmd" "$arg_1" "\$arg_2 skipped for size" "\$arg_3 skipped for size" "$arg_4" "${args[*]}" "$exit_code"

  return "$exit_code"
}
