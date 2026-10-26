# C002: In-session command toggle and split UI

[Main reference](../main.md#c002)

## Relevant transcript excerpts

<!-- workflow:excerpt C002 38 40 -->

[Transcript lines 38-40](../transcript.txt#L38-L40)

```text
   with `id world`. No content auto-detection. Long-term, agreed: every
   variation — raw/buffered × primary/alternate, split UI, dual channel —
   lands eventually, each for a different context (§13–§15). §7 is a concrete
```

<!-- workflow:excerpt C002 44 44 -->

[Transcript lines 44-44](../transcript.txt#L44-L44)

```text
   The web client already runs the "split UI" shape (screen + chat form).
```

<!-- workflow:excerpt C002 173 176 -->

[Transcript lines 173-176](../transcript.txt#L173-L176)

```text
- The browser client (`web/src/world.ts`) encodes keydowns to terminal bytes
  (`world-keyboard.ts:26-53`), sends `input`/`paste`/`resize` frames, and has
  a **separate chat form** (`world.ts:524-533`) — i.e. the web client already
  runs the split-UI shape.
```

<!-- workflow:excerpt C002 259 260 -->

[Transcript lines 259-260](../transcript.txt#L259-L260)

```text
- **Dual channel** — two live input paths in one SSH session. Not a mode
  switch, not a screen buffer choice.
```

<!-- workflow:excerpt C002 294 302 -->

[Transcript lines 294-302](../transcript.txt#L294-L302)

```text
### The four dual-channel shapes

1. **Split UI on one PTY.** World grid above, command line pinned at the
   bottom. One channel; the server draws both regions; keystrokes route by
   focus. **Gains:** one connection, one screen, no client changes, commands
   and play coexist visibly. **Losses:** the server owns more UI state; world
   redraw must not clobber the command line; focus state must be visible;
   round-trip latency for command echo. **Best for:** interactive SSH where
   you type `/records` mid-game. *The web client is already this shape.*
```

<!-- workflow:excerpt C002 311 316 -->

[Transcript lines 311-316](../transcript.txt#L311-L316)

```text
3. **Prefix key and a command buffer.** Raw everywhere; `Ctrl-P` (or `~ Enter`)
   switches to a line buffer in an on-demand prompt. **Gains:** smallest screen
   change, game stays dominant, no focus ambiguity when hidden. **Losses:**
   hidden mode state silently eats input; needs a visible indicator; escape
   handling gets fiddly. **Best for:** minimal change to the raw default —
   i.e. the later, optional in-session toggle.
```

<!-- workflow:excerpt C002 325 332 -->

[Transcript lines 325-332](../transcript.txt#L325-L332)

```text
| Trigger | Deterministic | Needs client support | Notes |
|---|---|---|---|
| SSH username prefix (`cmd:` like `explore:`) | yes | no | mirrors the existing explorer pattern; picks the mode at login |
| Startup flag or subcommand | yes | no | `id world join --command-mode` for the CLI client |
| Prefix key (`Ctrl-P`, `~ Enter`) | yes | no | the in-session toggle; needs visible mode indicator |
| Dedicated key (`Ctrl-]`, `Ctrl-\`) | yes | no | no collision with editing keys, undiscoverable |
| Type `/` in raw mode | no | no | a game that wants to type `/` breaks — do not |
| Ctrl+C / interrupt | ambiguous | no | already used for cancel; do not overload |
```

<!-- workflow:excerpt C002 335 337 -->

[Transcript lines 335-337](../transcript.txt#L335-L337)

```text
Two distinct switches: **session start** (line mode for automation/agents vs
raw interactive for humans) and **in-session** (entering/leaving command mode
while play stays up — the prefix-key and indicator work).
```

<!-- workflow:excerpt C002 345 347 -->

[Transcript lines 345-347](../transcript.txt#L345-L347)

```text
- **Buffered output to a region** — shape (1): the world redraws live while a
  bottom command region holds a buffered line the server knows about →
  server-side autocomplete without giving up the game.
```

<!-- workflow:excerpt C002 373 374 -->

[Transcript lines 373-374](../transcript.txt#L373-L374)

```text
3. **One trigger at first:** the login-time choice (SSH username prefix or CLI
   flag). Add the prefix-key toggle only if people want to switch mid-session.
```

<!-- workflow:excerpt C002 389 389 -->

[Transcript lines 389-389](../transcript.txt#L389-L389)

```text
| Power user mid-game | Raw + later toggle (shape 3) or split UI (shape 1) | Prefix key for a command, back to play |
```

<!-- workflow:excerpt C002 456 462 -->

[Transcript lines 456-462](../transcript.txt#L456-L462)

```text
### Step 3 — Optional later, in this order

1. **In-session toggle** (shape 3): prefix key intercepted in `SshIo::recv`
   only in raw mode; a visible mode line; `/quit` or the prefix key returns to
   raw. Only if users ask.
2. **Split UI** (shape 1): pinned command row + focus routing; much more
   server UI state; do it only with a concrete demand.
```

<!-- workflow:excerpt C002 657 657 -->

[Transcript lines 657-657](../transcript.txt#L657-L657)

```text
**Later:** the optional in-session toggle; `--json` transcript output for
```

<!-- workflow:excerpt C002 788 788 -->

[Transcript lines 788-788](../transcript.txt#L788-L788)

```text
4. **In-session toggle** — first step or later?
```

## Normalized purpose

Takes the two in-session dual-channel shapes: a prefix-key toggle into a command buffer with a visible mode indicator (shape 3) and a split UI with a pinned command row and focus routing on one PTY (shape 1), plus the web client's chat form as the existing split-UI precedent.

## Initial thoughts

- Shape 3, a prefix key with a visible indicator (L311-L316), is the later optional toggle; add it only if users ask (L373-L374, L458-L460).
- Shape 1, a split UI on one PTY (L296-L302), needs pinned-row redraw and focus routing; build it only with a concrete demand (L461-L462).
- The web client already runs the split-UI shape with a separate chat form (L173-L176; pkgs/id/web/src/world.ts:524-533).
- Rejected triggers: typing `/` in raw mode and overloading Ctrl+C (L331-L332). Session start and in-session are two separate switches (L335-L337).
- #535 tracks D1-045 (toggle, after #546) and D1-046 (split UI, after #562 and D1-045).

## Inferred suggestions

- Inference: The toggle's visible indicator and the split UI's command row could share one status-row renderer.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
