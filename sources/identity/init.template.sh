#!/usr/bin/env bash
# must be bash because we source a bashrc file
set -ex
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
/boot/dietpi/dietpi-software uninstall 103 104 # ramlog dropbear
/boot/dietpi/dietpi-software install 188 # go (git by dependency)
if [ -f /etc/bash.bashrc ]; then
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
cd code/src/identity
chmod +x *.ps1 *.sh
./build-libsql.ps1
get_http_status() {
    local url=$1
    curl -Lo /dev/null -s -w "%{http_code}\n" "$url"
}

start_time=$(date +%s)

set +e
while : ; do
    current_time=$(date +%s)
    elapsed_time=$((current_time - start_time))

    if [ "$elapsed_time" -ge 60 ]; then
        echo "1 minute has elapsed, stopping."
        break
    fi

    http_status=$(get_http_status "$CHARM_LINK_URL")
    echo "Checking URL: $CHARM_LINK_URL - HTTP status: $http_status"

    if [ "$http_status" -ne 000 ]; then
        echo "Verified charm link url is responding, breaking loop."
        break
    fi

    sleep 2
done
echo "Obtaining charm link"

response=$(curl -sL "$CHARM_LINK_URL" --data-urlencode "keys=$(./identity charm keys --simple | tr '\n' ',' | sed 's/,$//')")
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
./identity charm link -d "$CHARM_LINK"
./identity charm kv sync
./identity charm kv get dt.identity.init > .init
cat .init
chmod +x .init
echo "Running .init"
set +e
./.init
