https://github.com/nix-community/NixOS-WSL/releases/tag/22.05-5c211b47
https://github.com/nix-community/NixOS-WSL/releases/download/22.05-5c211b47/nixos-wsl-installer.tar.gz
https://github.com/nix-community/NixOS-WSL/releases/download/22.05-5c211b47/nixos-wsl-installer.tar.gz.sha256
```
PS C:\wsl> Get-History

  Id     Duration CommandLine
  --     -------- -----------
   1        6.829 wsl --import NixOS .\NixOS\ nixos-wsl-installer.tar.gz --version 2
   2       47.742 wsl -d NixOS
   3        1.986 wsl --shutdown
   4        0.046 wsl --list
   5       21.526 wsl -d NixOS
   6        0.053 wsl -s NixOS
   7        0.705 nix run github:Xe/gohello
   8     3:04.482 wsl
   9        0.729 wsl --shutdown
  10    26:18.340 wsl
```
