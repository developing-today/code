{
  ...
}:
{
  systemd.tmpfiles.rules = [
    "d /home/user/.config/oils 0755 user users"
    "d /home/user/.local/share/oils 0755 user group"
  ];
}
