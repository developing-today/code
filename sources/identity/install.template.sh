#!/usr/bin/env sh
CHARM_URL="{{CHARM_URL}}"
if [ -z "$CHARM_URL" ] || [ "$CHARM_URL" = "{{CHARM_URL}}" ]; then
  CHARM_URL="cloud.charm.sh"
fi
export CHARM_URL
CHARM_LINK_URL="{{CHARM_LINK_URL}}"
if [ -z "$CHARM_LINK_URL" ] || [ "$CHARM_LINK_URL" = "\{\{CHARM_LINK_URL\}\}" ]; then
  echo "No charm link provided"
  exit 1
fi
/boot/dietpi/dietpi-software uninstall 103 104 # ramlog dropbear
/boot/dietpi/dietpi-software install 188 # go (git by dependency)
source /etc/bash.bashrc
apt update
apt install -y wget libicu72 npm ucspi-tcp unzip
npm install -g npm@latest
wget https://github.com/PowerShell/PowerShell/releases/download/v7.4.1/powershell_7.4.1-1.deb_amd64.deb
dpkg -i powershell_7.4.1-1.deb_amd64.deb
apt install -f
cd ~$USER
if [ ! -d "code" ]; then
  git clone https://github.com/developing-today/code
else
  echo "code directory already exists"
fi
cd code/source/identity
chmod +x *.ps1
./build-libsql.ps1
CHARM_LINK=$(wget -qO- "$CHARM_LINK_URL")
if [ -z "$CHARM_LINK" ]; then
  echo "Failed to obtain charm link"
  exit 1
fi
./identity charm link "$CHARM_LINK"
./identity charm kv sync
./identity charm kv get dt.identity.init > init
chmod +x init
. ./init
