#!/usr/bin/env sh
set -eu
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
if [ -n "{{CHARM_LINK_URL}}" ] && [ "{{CHARM_LINK_URL}}" != "\{\{CHARM_LINK_URL\}\}" ]; then
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
  CHARM_LINK_URL="http://$IP:$PORT/link"
  echo "No charm link provided"
  echo "Using default link: $CHARM_LINK_URL"
fi
export CHARM_LINK_URL
/boot/dietpi/dietpi-software uninstall 103 104 # ramlog dropbear
/boot/dietpi/dietpi-software install 188 # go (git by dependency)
source /etc/bash.bashrc
mkdir -p /etc/apt/keyrings
curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key | gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg
echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_21.x nodistro main" | tee /etc/apt/sources.list.d/nodesource.list
apt update
apt install -y curl nodejs npm ucspi-tcp unzip
npm install -g npm@latest
npm --version
node --version
if command -v snap; then
  snap install powershell --classic
else
  apt install -y libicu72
  curl -O https://github.com/PowerShell/PowerShell/releases/download/v7.4.1/powershell_7.4.1-1.deb_amd64.deb
  dpkg -i powershell_7.4.1-1.deb_amd64.deb
  apt install -f
fi
cd ~$USER
if [ ! -d "code" ]; then
  git clone https://github.com/developing-today/code
else
  echo "code directory already exists"
fi
cd code/source/identity
chmod +x *.ps1
./build-libsql.ps1
CHARM_LINK=$(curl -sL "$CHARM_LINK_URL")
if [ -z "$CHARM_LINK" ]; then
  echo "Failed to obtain charm link"
  exit 1
fi
./identity charm link "$CHARM_LINK"
./identity charm kv sync
./identity charm kv get dt.identity.init > .init
chmod +x .init
./.init
