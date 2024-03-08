#!/usr/bin/env sh
dietpi-software install 188 # install latest go version (and git)
apt update
apt install -y wget
wget https://github.com/PowerShell/PowerShell/releases/download/v7.4.1/powershell_7.4.1-1.deb_amd64.deb
dpkg -i powershell_7.4.1-1.deb_amd64.deb
apt install -f
rm powershell_7.4.1-1.deb_amd64.deb
apt install npm
git clone https://github.com/developing-today/code
cd code/source/identity
chmod +x *.ps1
./start-server-all.ps1
