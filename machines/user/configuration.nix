{
  imports = [
    # contains your disk format and partitioning configuration.
    ../../modules/disko.nix
    # this file is shared among all machines
    ../../modules/shared.nix
    # enables GNOME desktop (optional)
    ../../modules/gnome.nix
  ];

  # This is your user login name.
  users.users.user.name = "user";

  # Set this for clan commands use ssh i.e. `clan machines update`
  # If you change the hostname, you need to update this line to root@<new-hostname>
  # This only works however if you have avahi running on your admin machine else use IP
  clan.core.networking.targetHost = "root@10.0.0.252";

  # You can get your disk id by running the following command on the installer:
  # Replace <IP> with the IP of the installer printed on the screen or by running the `ip addr` command.
  # ssh root@<IP> lsblk --output NAME,ID-LINK,FSTYPE,SIZE,MOUNTPOINT
  disko.devices.disk.main.device = "/dev/disk/by-id/nvme-eui.002538b9114010ec";

  # IMPORTANT! Add your SSH key here
  # e.g. > cat ~/.ssh/id_ed25519.pub
  users.users.root.openssh.authorizedKeys.keys = [
    ''
      ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIO3eHr75J9luI8W+f6FaOpdK37h+Tp+W2H18iJ7Azy00 user@amd
    ''
  ];

  # Zerotier needs one controller to accept new nodes. Once accepted
  # the controller can be offline and routing still works.
  clan.core.networking.zerotier.controller.enable = true;
}
