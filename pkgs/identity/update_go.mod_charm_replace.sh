#!/usr/bin/env bash

# Enable exit on error and pipefail to handle errors in piped commands
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

# Set the repository and branch
REPO="github.com/developing-today-forks/charm"
BRANCH="main"

# Fetch the latest commit hash from the main branch
echo "Fetching latest commit hash from ${REPO} on branch ${BRANCH}..."
LATEST_HASH=$(git ls-remote https://"${REPO}".git | grep "refs/heads/${BRANCH}$" | cut -f 1)

if [[ -z ${LATEST_HASH} ]]; then
  echo "Failed to fetch the latest commit hash. Exiting..."
  exit 1
fi

echo "Latest commit hash: ${LATEST_HASH}"

# Update go.mod replace directive
echo "Updating go.mod to use the latest commit..."
go mod edit -replace="github.com/charmbracelet/charm=${REPO}@${LATEST_HASH}"

# Tidy and verify
echo "Tidying and verifying module..."
go mod tidy
go mod verify

echo "Update complete."
