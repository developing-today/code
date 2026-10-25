# SSH command grammar, the `id world join` pump, and `--command-mode`

Short plan for issue #546: one line grammar for the three surfaces that already
each know a different subset — SSH command mode, the `id world join` stdin
pump, and the CLI one-off verbs.

## Goal

The same sentence — `chat`, `records`, `stats`, `logs`, `caps`, `quit` — works
as an SSH command (`/chat` … or a plain line), a piped script line, and a CLI
one-off, so the three surfaces stop drifting.

## Approach

1. **One shared parser.** A new `world_line` module parses one line into a
   `WorldLine` action with no surface knowledge. It understands the `/`-prefixed
   commands (help, quit, records, documents, document, input, stats, logs,
   caps) and treats any other non-empty line as chat.

2. **SSH command mode uses the parser.** `world_ssh::world_command` becomes a
   thin wrapper over `world_line::parse`, gaining `/stats`, `/logs` and `/caps`
   on top of the existing `/help`, `/quit`, `/records`, `/documents`,
   `/document` and `/input`.

3. **The join pump uses the parser behind `--command-mode`.** `id world join`
   gains a `--command-mode` flag. With it set, each stdin line is parsed by the
   same grammar and mapped to the session frame it needs (chat, input, records,
   stats, caps, logs, quit). Without the flag, today's behaviour is unchanged
   (`/quit`, `/input`, everything else chat).

4. **Tests.** One test table asserts the same line yields the same action on
   SSH, the pump and the CLI.

## Not in scope

- New capability providers; `/stats`, `/logs` and `/caps` reuse the world's
  existing info, event-tail and caps-report paths.
- A structured (non-line) command framing; that is a later input-mode variant
  per the [SSH input-modes decision](../../../doc/2026-10-24T20-18-37Z_decision_ssh_input_modes/2026-10-24T20-18-37Z_decision_ssh_input_modes.md).