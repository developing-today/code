# docs

to be written
flake.nix
-> hosts
hosts/<host>/(<user>)
-> home
-> secrets/sops/<host>/(<user>)
home/<user>/<host>
-> secrets/sops/<user>/(<host>)
lib/.sops.yaml
-> secrets/sops/
-> secrets/sops/<host>/(<user>)
-> secrets/sops/<user>/(<host>)
secrets/sops/<host>/(<user>)
secrets/sops/<user>/(<host>)

- take apart configuration.nix
  - first make all the parts into files and imports in configuration.nix
  - then move the files into common,global,hosts,home,etc.

difference between lib and module?
difference between created module and configured module?
difference between lib and pkgs?
difference between pkgs and modules?
do overlays need a root folder?
how to best handle many-to-many relationships?
if hostname is unique, how to handle template? shell script to make <name>\_<random>?
if hostname is not unique, how to handle discovery/dns/networking/vpn/ssh?
setup persistence, disko, iso-installer by default
setup vpn (tailscale for now, later also wireguard)
setup some kind of monitoring
setup ci/cd/build-farm
setup service discovery (vpn names and gokrazy/caddy??)
finish network setup, flash routers(update firmware, install os (r7-router or openwrt?)), build rails, rack up switches, plug it all in, ensure latest firmware/onie for switches
