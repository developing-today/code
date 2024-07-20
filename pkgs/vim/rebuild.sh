#!/usr/bin/env bash
set -xeuo pipefail
#ulimit -n $(ulimit -Hn)
#sudo prlimit --pid $$ --nofile=1000000:1000000
# Get the directory of the script
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Git add for the script's directory
cd "${script_dir}" || exit 1
echo "entered: ${script_dir}"
echo "git add ."
git add .

# Loop through each directory in config
for dir in "${script_dir}"/config/*; do
  if [[ -d ${dir} ]]; then
    #echo "is a dir: ${dir}"
    # Skip the directory if it doesn't contain a flake.nom file
    if [[ ! -f "${dir}/flake.nix" ]] && [[ ! -f "${dir}/rebuild.sh" ]]; then
      #echo "not a flake: ${dir}"
      #echo "continue"
      continue
    fi
    cd "${dir}" || exit 1
    echo "entered: ${dir}"
    # If a rebuild --json script exists, execute it
    if [[ -f "./rebuild.sh" ]]; then
      echo "./rebuild.sh exists, executing..."
      chmod +x ./rebuild.sh
      ./rebuild.sh
      #else
      #echo "./rebuild.sh does not exist"
    fi
    if [[ -f "./flake.nix" ]]; then
      echo "is a flake: ${dir}"
      echo "updating flake..."
      nix flake update --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace |& nom --json
      echo "building flake ..."
      nix build --json --print-out-paths --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace |& nom --json
      #nom build --json --print-out-paths --json --print-build-logs --verbose --keep-going --log-format internal-json --fallback   --show-trace --json |
      #  jq -r '.[].outputs | to_entries[].value' |
      #  cachix push binary
      # TODO: skip cachix if not setup
      #nom flake archive --print-build-logs --verbose --keep-going --log-format internal-json --fallback  --show-trace --json | jq -r '.path,(.inputs|to_entries[].value.path)' | cachix push binary # todo: make optional
      echo "exiting: ${dir}"
      cd "${script_dir}" || exit 1
      echo "entered: ${script_dir}"
      #else
      #echo "not a flake: ${dir}"
    fi
  #else
  #echo "not a dir: ${dir}"
  fi
done

echo "git add ."
git add .

if [[ -f "./flake.nix" ]]; then
  echo "is a flake: ${script_dir}"
  # TODO: sometimes do update-ref instead of update
  echo "updating flake..."
  nix flake update --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace |& nom --json
  # TODO: skip cachix if not setup
  echo "building flake..."
  nix build --json --print-out-paths --json --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace |& nom --json
  #nom build --json --print-out-paths --json --print-build-logs --verbose --keep-going --log-format internal-json --fallback   --show-trace --json |
  #  jq -r '.[].outputs | to_entries[].value' |
  #  cachix push binary
  #nom flake archive --print-build-logs --verbose --keep-going --log-format internal-json --fallback   --show-trace --json |
  #  jq -r '.path,(.inputs|to_entries[].value.path)' |
  #  cachix push binary # todo: make optional
  echo "git add ."
  git add .
else
  echo "not a flake: ${script_dir}"
fi
echo "done: ${script_dir}"
