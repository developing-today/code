#!/usr/bin/env sh
/boot/dietpi/dietpi-software uninstall 104 # dropbear
/boot/dietpi/dietpi-software install 0 188 # openssh-client go (and git by dependency)
source /etc/bash.bashrc # to get go in the path
apt update # to clear out any ghosts
apt install -y wget libicu72 npm # wget libicu72 for powershell, npm for htmx/tailwind
npm install -g npm@latest # update npm because apt installs an old version
wget https://github.com/PowerShell/PowerShell/releases/download/v7.4.1/powershell_7.4.1-1.deb_amd64.deb # apt doesn't have powershell so add it
dpkg -i powershell_7.4.1-1.deb_amd64.deb # install powershell
apt install -f # fix any dependencies
cd ~$USER # go back to home directory for some reason
if [ ! -d "code" ]; then
  git clone https://github.com/developing-today/code # clone the code repo
else
  echo "code directory already exists" # else, don't clone it
fi
cd code/source/identity # go to the identity directory (here)
chmod +x *.ps1 # ensure the powershell scripts are executable
./build-libsql.ps1 # build the libsql binary ahead of start to start up secrets
# ./identity charm link $_ # link to host with secrets
./identity charm kv sync # sync badgerdb
./identity charm kv get dt.identity.init > init
chmod +x init # make the init script executable
./init # run the init script

# charm kv set dt.identity.init @"
# !/usr/bin/env pwsh
# cd ~`$USER/code/source/identity
# ./identity charm kv sync
# ./identity charm kv get dt.identity.init > init
# ./start-server-all.ps1
# "@
