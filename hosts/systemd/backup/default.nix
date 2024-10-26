{
  ...
}:
{
  systemd.tmpfiles.rules = [
    "d /home/backup/.config/oils 0755 user users"
    "d /home/backup/.local/share/oils 0755 user group"
  ];
}
