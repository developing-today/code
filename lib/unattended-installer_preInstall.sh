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

echo "Sleep Seconds: $sleep_time"
echo "Force: $force"

echo "preInstall starting"

echo "Listing /iso/bootstrap..."
ls -lahR /iso/bootstrap
echo "Done listing /iso/bootstrap"

echo "mkdir /mnt/nix/persistent"
mkdir -p /mnt/nix/persistent
echo "/mnt/nix/persistent created"

echo "Copying /iso/bootstrap into /mnt/nix/persistent..."
cp -LRv /iso/bootstrap /mnt/nix/persistent
echo "Done copying /iso/bootstrap into /mnt/nix/persistent"

echo "Listing /mnt/nix/persistent..."
ls -lahR /mnt/nix/persistent
echo "Done listing /mnt/nix/persistent"

echo "Copying /mnt/nix/persistent/bootstrap/etc into /mnt/nix/persistent..."
cp -LRv /mnt/nix/persistent/bootstrap/etc /mnt/nix/persistent
echo "Done copying /mnt/nix/persistent/bootstrap/etc into /mnt/nix/persistent"

echo "Listing /mnt/nix/persistent..."
ls -lahR /mnt/nix/persistent
echo "Done listing /mnt/nix/persistent"

# echo "Uncompressing all .tar.gz files in /mnt/bootstrap..."
# find /mnt/nix/persistent/bootstrap -name "*.tar.gz" -exec sh -c '
#     dir=$(dirname "$1")
#     base=$(basename "$1" .tar.gz)
#     mkdir -p "$dir/$base"
#     tar -xzf "$1" -C "$dir/$base"
# ' sh {} \;
# echo "Done uncompressing all .tar.gz files in /mnt/bootstrap"

if [ "$force" = true ]; then
  echo "Force flag detected. Skipping sleep."
else
  echo "Success! Sleeping ${sleep_time} seconds..."
  set +x
  while [ $sleep_time -gt 0 ]; do
    echo -ne "\r\033[K$sleep_time\n"
    sleep 1
    sleep_time=$(($sleep_time - 1))
  done
  set -x
  echo "Done waiting ${sleep_time} seconds."
fi

echo "preInstall done"
