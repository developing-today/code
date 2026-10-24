# SSH input modes: raw by default, `cmd:` line mode, and the username grammar

Decision record for the SSH presentation of a hosted world. Fixes the gap
flagged in the platform world-state report (RPT §8 "Input modes — not in
either plan"): the input-mode decision and the username grammar existed only
in uncommitted reports.

## Decision

1. **Raw keystrokes are the default.** When an SSH user logs in with a plain
   world name (or an empty name for the default world), the session runs in
   _raw_ mode: every keystroke becomes an `input` frame and each `view` frame
   repaints the terminal, so an ordinary `ssh` client can play any world that
   presents a view. The alternate screen buffer is used in raw mode only.

2. **`cmd:` line mode is the explicit login-time entry.** Prefixing the SSH
   username with `cmd:` enters _command_ (line) mode, where each line is parsed
   as a command instead of forwarded as an input keystroke. The grammar mirrors
   the `id world` subcommands: chat, records, stats, logs, caps, and quit. See
   the grammar task (below) and the `world_ssh` module.

3. **Other variations come later.** Additional input modes (scripted pipes,
   structured command framing, and any further username prefixes) are future
   work and are not part of this decision. They will reuse the same line
   grammar and the same username-prefix mechanism.

## Username grammar

The SSH username selects the world and the mode, in one string:

| Username        | Meaning                                                   |
| --------------- | --------------------------------------------------------- |
| `world-name`    | Join `world-name` in raw mode (default world when empty). |
| `explore:world` | Read-only explore session for `world` (public-key login). |
| `cmd:world`     | Join `world` in command (line) mode.                      |

The SSH password is the join capability. `explore:` is the only prefix that
supports public-key login; `cmd:` and the plain name use password auth.

## References

- [Phase 11 plan](../../.opencode/plans/phase-11-foundations.md)
- [Phase 12 plan](../../.opencode/plans/phase-12-platform-flexibility.md)
- [SSH command grammar issue](https://github.com/developing-today/code/issues/546)
