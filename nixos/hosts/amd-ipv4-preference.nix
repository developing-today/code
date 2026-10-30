_:
{
  # Temporary workaround for the home IPv6 uplink stalling on payload
  # transfers (developing-today/code#1280): prefer IPv4 in getaddrinfo
  # result sorting until the home IPv6 path is repaired. Remove with
  # the issue.
  networking.getaddrinfo = {
    label = {
      "::1/128" = 0;
      "::/0" = 1;
      "2002::/16" = 2;
      "::/96" = 3;
      "::ffff:0:0/96" = 4;
    };
    precedence = {
      "::1/128" = 50;
      "::/0" = 40;
      "2002::/16" = 30;
      "::/96" = 20;
      "::ffff:0:0/96" = 100;
    };
  };
}
