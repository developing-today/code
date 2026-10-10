# TODO

- Replace the takeover handoff with a supervisor that owns each stdio child's pipes for the child's whole life. `mcpx daemon --takeover` is a stopgap: the old daemon passes its children's pipes to its successor over a unix socket, so every handoff depends on the framing, the ack sequence and the restore path working. A supervisor that outlives the daemon would make a daemon upgrade a plain restart of the front end.
