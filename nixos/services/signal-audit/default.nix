# Declarative Linux audit for signal attribution (code#8787, follow-up
# to code#8777). The 2026-11-11 SIGTERM that killed pm-aggregator and
# pm-dispatch left no record of its sender: systemd logs the death,
# never the sender. These rules record the kill-family syscalls with
# the sender's auid/exe/pid/comm under key signal-attribution, so
# `ausearch -k signal-attribution` answers "who sent this signal".
# The pm units keep their sigterm-witness wrapper, which survives if
# auditd is ever dropped.
{
  security.auditd.enable = true;
  security.audit.rules = [
    "-a always,exit -F arch=b64 -S kill -S tkill -S tgkill -k signal-attribution"
  ];
}
