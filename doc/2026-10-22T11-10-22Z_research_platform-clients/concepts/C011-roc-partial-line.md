# C011: Roc partial-line surface

[Main reference](../main.md#c011)

## Relevant transcript excerpts

<!-- workflow:excerpt C011 107 110 -->

[Transcript lines 107-110](../transcript.txt#L107-L110)

```text
- **`platform.roc`** requires `layout : model, U64, U64 -> Str` (guest-aware
  reflow). `Key.parse` decodes raw terminal input bytes into a `Key` tag union
  (`Char/Ctrl/Enter/Tab/Backspace/Escape/Delete/arrows/Home/End/PageUp/
  PageDown/Unknown`) matching the browser encoder's sequences.
```

<!-- workflow:excerpt C011 142 155 -->

[Transcript lines 142-155](../transcript.txt#L142-L155)

```text
3. **`ClientFrame::Input`** (`world_session.rs:233-235`) is hex-decoded and
   dispatched in `handle_client_frame` (`world_session.rs:2256-2468`) to
   `world.input(token, data)`; `paste` (≤4096 bytes, no control chars) and
   `chat` take sibling paths; `resize` (1..=500 × 1..=200) is **ephemeral,
   never journaled** and re-sends a fitted `view`.
4. **The journal** stores `WorldEventKind::Input(Vec<u8>)` as **opaque bytes**
   (`world.rs:275`, validation `world.rs:1649-1667`); a rejected input is
   never journaled.
5. **The guest** receives the bytes as **valid UTF-8 text**
   (`sandbox.rs:677`) copied into linear memory and passed to
   `plaza_update(model, participant, ptr, len)` (`sandbox.rs:343-358`).
   `System` host events (presence, capability results, ticks) arrive the same
   way with participant 0 (`sandbox.rs:683-685`); `Chat` and `ProgramInstalled`
   are **not** delivered to the guest. Key parsing is guest-side (`Key.parse`).
```

<!-- workflow:excerpt C011 271 274 -->

[Transcript lines 271-274](../transcript.txt#L271-L274)

```text
Local echo is snappier but the server does not know what is typed, so
autocomplete and validation need streaming anyway. Server echo gives the
server everything as typed, at one round trip per keystroke unless batched.
The hybrid modern TUI line editors use over SSH: stream keystrokes for
```

<!-- workflow:excerpt C011 345 351 -->

[Transcript lines 345-351](../transcript.txt#L345-L351)

```text
- **Buffered output to a region** — shape (1): the world redraws live while a
  bottom command region holds a buffered line the server knows about →
  server-side autocomplete without giving up the game.
- **Extended to Roc apps** — pass "the viewer is typing this partial line" to
  the program; a Roc app could render live autocomplete or a validating form.
  New guest-facing surface; needs its own design: what event carries the
  partial line, that it is **not** journaled, and its bounds.
```

<!-- workflow:excerpt C011 375 376 -->

[Transcript lines 375-376](../transcript.txt#L375-L376)

```text
4. **Defer in-session buffered typing (the Roc partial-line surface)** until
   the line mode exists and the command grammar is known.
```

<!-- workflow:excerpt C011 443 446 -->

[Transcript lines 443-446](../transcript.txt#L443-L446)

```text
- **Bounds and journaling:** line buffer ≤1024 bytes (PR #211's discard rule);
  document 64 KiB / list 512 items / 32 KiB; records total cap new. Chat and
  input lines become durable events (existing paths); **partial lines and
  mode state are never journaled** (same rule as `resize`).
```

<!-- workflow:excerpt C011 465 466 -->

[Transcript lines 465-466](../transcript.txt#L465-L466)

```text
4. **Roc partial-line surface** (buffered typing to guests): new non-journaled
   notification, bounded; needs its own design record.
```

<!-- workflow:excerpt C011 658 658 -->

[Transcript lines 658-658](../transcript.txt#L658-L658)

```text
agents; documents as a protocol frame pair; the Roc partial-line surface; then
```

## Normalized purpose

Takes the guest-facing extension: a bounded, never-journaled notification telling a Roc program what the viewer is typing, so apps can render live autocomplete or a validating form, and how guest input and host events reach programs today.

## Initial thoughts

- Pass 'the viewer is typing this partial line' to the program, so a Roc app can render live autocomplete or a validating form (L348-L350).
- It needs its own design: which event carries the line, that it is never journaled, and its bounds (L350-L351, L465-L466).
- Precedent: resize is ephemeral and never journaled (L145-L146); partial lines and mode state are never journaled (L445-L446).
- Deferred until the line mode exists and the grammar is known (L375-L376). #535 tracks D1-048 after D1-046 (split UI) and F12.7.

## Inferred suggestions

- Inference: Partial-line notifications are high-frequency; coalesce them like host events (L162-L165) and bound their rate.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
