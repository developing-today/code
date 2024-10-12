#!/usr/bin/env bash
set -e #-o pipefail

# use care, this is copied as a string.
# you can't use variable inputs.
# instead, set template variables.
# update the module to replace the
# template variables at build time.

cleanup() {
  echo -e "\nScript interrupted. Exiting..."
  exit 1
}
trap cleanup SIGINT

# default_command="reboot"
default_command="poweroff"
command="$default_command"
sleep_time=30
force=false

print_usage() {
  echo "Usage: $0 [command] [-t|--time <seconds>] [-f|--force]"
  echo "  command: Command to execute (default: poweroff)"
  echo "  -t, --time: Set the countdown time in seconds (default: 30)"
}

process_args() {
  while [[ $# -gt 0 ]]; do
    case $1 in
      -t|--time)
        if [[ -n $2 && $2 =~ ^[0-9]+$ ]]; then
          sleep_time=$2
          shift 2
        else
          echo "Error: --time requires a numeric argument"
          print_usage
          exit 1
        fi
        ;;
      -h|--help)
        print_usage
        exit 0
        ;;
      -*)
        echo "Error: Unknown option $1"
        print_usage
        exit 1
        ;;
      *)
        if [[ -z "$command" || "$command" == "$default_command" ]]; then
          command="$1"
        else
          echo "Error: Unexpected argument $1"
          print_usage
          exit 1
        fi
        shift
        ;;
    esac
  done
}

echo "Processing args"
process_args "$@"
echo "Processed args"

echo "Command:"
echo "$command"
echo "Sleep Seconds: $sleep_time"
echo "Force: $force"

if [ "$force" = true ]; then
  echo "Force flag detected. Executing '${command}' immediately..."
else
  echo "Success! Executing '${command}' in ${sleep_time} seconds..."
  set +x
  while [ $sleep_time -gt 0 ]; do
    echo -ne "\r\033[K$sleep_time\n"
    sleep 1
    sleep_time=$(($sleep_time - 1))
  done
  set -x
  echo "Done waiting ${sleep_time} seconds."
fi

echo -e "\nExecuting '${command}'...\n"
# $command
