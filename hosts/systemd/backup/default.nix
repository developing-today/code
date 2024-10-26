{
  ...
}:
{
  systemd.tmpfiles.rules = [
    # oils home manager program?
    # oils nixos program?
    "d /home/backup/.config/oils 0755 user users"
    "f /home/backup/.local/share/oils/osh_history 0644 user users"
  ];
}
