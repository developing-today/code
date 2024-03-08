#!/usr/bin/env sh
/boot/dietpi/dietpi-software install 188 # install latest go version (and git)
apt update
apt install -y wget
wget https://github.com/PowerShell/PowerShell/releases/download/v7.4.1/powershell_7.4.1-1.deb_amd64.deb
dpkg -i powershell_7.4.1-1.deb_amd64.deb
apt install -f
rm powershell_7.4.1-1.deb_amd64.deb
# curl -fsSL https://deb.nodesource.com/setup_21.x | bash - &&\
# apt-get install -y nodejs
apt install npm
npm install -g npm@latest

cd ~$USER
if [ ! -d "code" ]; then
  git clone https://github.com/developing-today/code
else
  echo "code directory already exists"
fi
cd code/source/identity
chmod +x *.ps1

./start-server-all.ps1
