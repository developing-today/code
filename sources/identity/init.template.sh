#!/usr/bin/env bash
# must be bash because we source a bashrc file
set -ex
INIT_PATH="./.init.$(date +%s)"
LOG_PATH="$INIT_PATH.log"
if [ -n "$1" ]; then
  CHARM_URL="$1"
fi
if [ -n "{{CHARM_URL}}" ] && [ "{{CHARM_URL}}" != "\{\{CHARM_URL\}\}" ]; then
  CHARM_URL="{{CHARM_URL}}"
fi
if [ -z "$CHARM_URL" ] || [ "$CHARM_URL" = "\{\{CHARM_URL\}\}" ]; then
  CHARM_URL="cloud.charm.sh"
  echo "No charm url provided"
  echo "Using default url: $CHARM_URL"
fi
export CHARM_URL
if [ -n "$2" ]; then
  CHARM_LINK_URL="$2"
fi
if [ -z "$CHARM_LINK_URL" ] && [ -n "{{CHARM_LINK_URL}}" ] && [ "{{CHARM_LINK_URL}}" != "\{\{CHARM_LINK_URL\}\}" ]; then
  CHARM_LINK_URL="{{CHARM_LINK_URL}}"
fi
if [ -z "$CHARM_LINK_URL" ] || [ "$CHARM_LINK_URL" = "\{\{CHARM_LINK_URL\}\}" ]; then
  IP=$(hostname -I | awk '{print $1}')
  if [ "$(expr substr "$IP" 1 4)" = "172." ]; then
    IP=$(hostname -I | awk '{print $2}')
    if [ "$(expr substr "$IP" 1 4)" = "172." ]; then
      IP="127.0.0.1"
    fi
  fi
  PORT=3333
  CHARM_LINK_URL="http://$IP:$PORT/link"
  echo "No charm link provided"
  echo "Using default link: $CHARM_LINK_URL"
fi
export CHARM_LINK_URL
# if [ -n "$3" ]; then
#   CHARM_DATA_DIR="$3"
# fi
# CHARM_DATA_DIR="${CHARM_DATA_DIR:-./data/charm/init}"
REPO_ROOT=$(git rev-parse --show-toplevel)
CHARM_DATA_DIR="$REPO_ROOT/sources/identity/data/charm/consumer"
echo "CHARM_DATA_DIR: $CHARM_DATA_DIR"
if [ -z "$NO_INSTALL" ]; then
  set +e
  /boot/dietpi/dietpi-software uninstall 103 104 # ramlog dropbear
  /boot/dietpi/dietpi-software install 188 # go (git by dependency)
  set -e
  if [ -f /etc/bash.bashrc ]; then
    echo "Sourcing /etc/bash.bashrc"
    source /etc/bash.bashrc
  else
    echo "/etc/bash.bashrc does not exist, continuing without sourcing it."
  fi
  mkdir -p /etc/apt/keyrings
  curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key | gpg --batch --yes --dearmor -o /etc/apt/keyrings/nodesource.gpg
  echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_21.x nodistro main" | tee /etc/apt/sources.list.d/nodesource.list
  DEBIAN_FRONTEND=noninteractive apt update
  DEBIAN_FRONTEND=noninteractive apt dist-upgrade -yq
  DEBIAN_FRONTEND=noninteractive apt autoremove -y
  DEBIAN_FRONTEND=noninteractive apt autoclean -y
  DEBIAN_FRONTEND=noninteractive apt install -y curl nodejs ucspi-tcp unzip xxd unattended-upgrades
  AUTO_UPGRADES_FILE="/etc/apt/apt.conf.d/20auto-upgrades"
  REQUIRED_LINES=(
      'APT::Periodic::Update-Package-Lists "1";'
      'APT::Periodic::Download-Upgradeable-Packages "1";'
      'APT::Periodic::AutocleanInterval "7";'
      'APT::Periodic::Unattended-Upgrade "1";'
  )
  add_line_if_not_present() {
      local line="$1"
      local file="$2"
      grep -qF -- "$line" "$file" || echo "$line" >> "$file"
  }
  if [ ! -f "$AUTO_UPGRADES_FILE" ]; then
      echo "$AUTO_UPGRADES_FILE does not exist, creating it..."
      touch "$AUTO_UPGRADES_FILE"
  fi
  for line in "${REQUIRED_LINES[@]}"; do
      add_line_if_not_present "$line" "$AUTO_UPGRADES_FILE"
  done
  echo "The $AUTO_UPGRADES_FILE has been updated."

  npm install -g npm@latest
  npm --version
  node --version
  if command -v snap; then
    snap install powershell --classic
  else
    DEBIAN_FRONTEND=noninteractive apt install -y libicu72
    curl -LO https://github.com/PowerShell/PowerShell/releases/download/v7.4.1/powershell_7.4.1-1.deb_amd64.deb
    dpkg -i powershell_7.4.1-1.deb_amd64.deb
    DEBIAN_FRONTEND=noninteractive apt install -f
  fi
  cd ~
  if [ ! -d "code" ]; then
    git clone https://github.com/developing-today/code
  else
    echo "code directory already exists"
  fi
  cd code/sources/identity
  chmod +x *.ps1 *.sh
  ./build-libsql.ps1
fi
get_http_status() {
    local url=$1
    curl -Lo /dev/null -s -w "%{http_code}\n" "$url"
}
start_time=$(date +%s)
CHARM_DATA_DIR="$REPO_ROOT/sources/identity/data/charm/consumer" $REPO_ROOT/sources/identity/identity charm id

set +ex
while : ; do
    current_time=$(date +%s)
    elapsed_time=$((current_time - start_time))

    if [ "$elapsed_time" -ge 60 ]; then
        echo "1 minute has elapsed, stopping."
        break
    fi

    http_status=$(get_http_status "$CHARM_LINK_URL")
    echo "Checking URL: $CHARM_LINK_URL - HTTP status: $http_status - Elapsed time: $elapsed_time"

    if [ "$http_status" -ne 000 ]; then
        echo "Verified charm link url is responding, breaking loop."
        break
    fi

    sleep 2
done
set -x
echo "Obtaining charm link"

response=$(curl -sL "$CHARM_LINK_URL" --data-urlencode "keys=$(CHARM_DATA_DIR="$REPO_ROOT/sources/identity/data/charm/consumer" ./identity charm keys --simple | tr '\n' ',' | sed 's/,$//')")
LAST_EXIT_CODE=$?
if [ "$LAST_EXIT_CODE" -ne 0 ]; then
    echo "Failed to obtain charm link"
    exit 1
fi
set -e
if [ -n "$response" ]; then
    extracted_value=$(echo "$response" | sed -n 's/.*HTTP\/1\.1 200 \(.*\)\r.*/\1/p')

    if [ -z "$extracted_value" ]; then
        echo "Unexpected response: $extracted_value"
        exit 1
    fi
else
    echo "Failed to obtain charm link"
    exit 1
fi
set -ex
CHARM_LINK=$extracted_value
CHARM_DATA_DIR="$REPO_ROOT/sources/identity/data/charm/consumer" $REPO_ROOT/sources/identity/identity charm fs rm "charm:dt/identity/init/init" 2>/dev/null || true # is this wrong?
CHARM_DATA_DIR="$REPO_ROOT/sources/identity/data/charm/consumer" $REPO_ROOT/sources/identity/identity charm link -d "$CHARM_LINK"
CHARM_DATA_DIR="$REPO_ROOT/sources/identity/data/charm/consumer" $REPO_ROOT/sources/identity/identity charm id
START_TIME=$SECONDS
TIMEOUT=30

while : ; do
    LINE_COUNT=$(CHARM_DATA_DIR="$REPO_ROOT/sources/identity/data/charm/consumer" $REPO_ROOT/sources/identity/identity charm fs tree "charm:dt/identity/init" | wc -l)

    if [ "$LINE_COUNT" -gt 1 ]; then
        echo "Output has more than one line."
        break
    fi

    ELAPSED_TIME=$(( SECONDS - START_TIME ))
    if [ "$ELAPSED_TIME" -ge "$TIMEOUT" ]; then
        echo "Timeout reached, exiting."
        exit 1
    fi

    sleep 1
done
# CHARM_DATA_DIR="$REPO_ROOT/sources/identity/data/charm/consumer" $REPO_ROOT/sources/identity/identity charm kv list
CHARM_DATA_DIR="$REPO_ROOT/sources/identity/data/charm/consumer" $REPO_ROOT/sources/identity/identity charm fs cat "charm:dt/identity/init/init" >"$INIT_PATH"
if [ ! -f "$INIT_PATH" ]; then
  echo "No init script found at $INIT_PATH"
  exit 1
fi
INIT=$(cat "$INIT_PATH")
if [ -z "$INIT" ]; then
  echo "No init script found at $INIT_PATH"
  exit 1
fi
chmod +x "$INIT_PATH"
echo "Running init script at $INIT_PATH"
"$INIT_PATH"
