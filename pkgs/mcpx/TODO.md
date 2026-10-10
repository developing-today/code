# TODO

- Replace the takeover handoff with a supervisor that owns each stdio child's pipes for the child's whole life. `mcpx upgrade` (run from the new binary) and `mcpx daemon --takeover` are a stopgap: the old daemon starts the successor as its own child, so it stays in the service's cgroup, and passes its children's pipes to it over a unix socket. Every handoff depends on the framing, the ack sequence and the restore path working. A supervisor that outlives the daemon would make a daemon upgrade a plain restart of the front end.
