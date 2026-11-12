# TODO

- Replace the takeover handoff with a supervisor that owns each stdio child's pipes for the child's whole life. The current handoff is a stopgap: `mcpx upgrade` (run from the new binary) and `mcpx daemon --takeover` have the old daemon start the successor as its own child, so it stays in the service's cgroup, and pass its children's pipes to it over a unix socket. Every handoff depends on the framing, the ack sequence and the restore path working. A supervisor that outlives the daemon would make a daemon upgrade a plain restart of the front end.
- Streamable HTTP upstream sessions survive a takeover; SSE upstream sessions do not, because their standalone GET stream cannot be moved between processes. Those are closed and re-initialised on the successor's first call.
- A cold restart resumes legacy streamable HTTP upstream sessions from `upstream-sessions.json` (#1415, Phase 1 of #1414). Still open: record removal when a server leaves the config, and the rest of Phase 2 (#1416).
