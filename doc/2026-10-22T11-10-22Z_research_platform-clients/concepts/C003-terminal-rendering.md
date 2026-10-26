# C003: Alternate screen, incremental rendering and redraw batching

[Main reference](../main.md#c003)

## Relevant transcript excerpts

<!-- workflow:excerpt C003 38 40 -->

[Transcript lines 38-40](../transcript.txt#L38-L40)

```text
   with `id world`. No content auto-detection. Long-term, agreed: every
   variation — raw/buffered × primary/alternate, split UI, dual channel —
   lands eventually, each for a different context (§13–§15). §7 is a concrete
```

<!-- workflow:excerpt C003 157 172 -->

[Transcript lines 157-172](../transcript.txt#L157-L172)

```text
Output path and batching:

- Every committed event → each session immediately sends `Event` **plus a full
  `View`** (`world_session.rs:2110-2115`); `send_view` re-renders the whole
  presentation fitted to the viewport (`world_session.rs:2166-2209`).
- Host events signal a `tokio::watch` that **coalesces bursts** to one repaint
  per wake-up (`world.rs:2033, 2124-2125, 3117-3122`); a lagging broadcast
  subscriber resnapshots instead of receiving the backlog
  (`world_session.rs:2117-2126`).
- **No output batching otherwise.** SSH writes each frame immediately as a
  **full repaint on the primary screen** — `\x1b[H\x1b[2J` + SGR row runs via
  `ScreenSurface::render_ansi` (`world_presentation.rs:321-405`). There is
  **no alternate screen** (`\x1b[?1049` appears nowhere in the tree), no
  cursor save/restore, no incremental diffing. `error` frames print
  control-stripped text (`world_ssh.rs:681-686`) so no participant can inject
  escapes into another's terminal.
```

<!-- workflow:excerpt C003 254 258 -->

[Transcript lines 254-258](../transcript.txt#L254-L258)

```text
- **Terminal screen buffer** — primary (scrollback) versus alternate
  (full-screen app). Display, not input. (`main` today uses neither properly:
  primary screen with clear-and-repaint, so scrollback accumulates cleared
  frames. Moving the interactive default to the alternate screen is an
  improvement to make, not the status quo.)
```

<!-- workflow:excerpt C003 279 292 -->

[Transcript lines 279-292](../transcript.txt#L279-L292)

```text
### Raw keystrokes in primary mode with scrollback

Yes — input routing is independent of the screen buffer. Three render styles:

- **Alternate screen** — the normal choice for a game. No scrollback,
  full-screen redraw. (`\x1b[?1049h/l` around the session.)
- **Primary screen, in-place redraw** — works; scrollback preserved above;
  write at the cursor, move back up.
- **Primary screen, appended log output** — raw input works but the display
  becomes a transcript. Fine for chat or log-following worlds.

The real cost of primary mode is that every redraw competes with scrollback
and anything else printed — which is exactly today's clear-and-repaint
behavior.
```

<!-- workflow:excerpt C003 339 344 -->

[Transcript lines 339-344](../transcript.txt#L339-L344)

```text
### Buffered output while staying interactive

Separate axis from input routing:

- **Batching** — accumulate redraws for a few milliseconds, send one frame.
  Server-side; no client change. (Today: one full repaint per event.)
```

<!-- workflow:excerpt C003 369 370 -->

[Transcript lines 369-370](../transcript.txt#L369-L370)

```text
1. **Raw keystrokes stay the interactive default.** Consider moving the
   renderer to the alternate screen as a small, separate improvement.
```

<!-- workflow:excerpt C003 518 519 -->

[Transcript lines 518-519](../transcript.txt#L518-L519)

```text
  non-blocking); F11.9 bytes-per-frame table (80×24 and 200×60 per example —
  the baseline F12.11 must beat).
```

<!-- workflow:excerpt C003 544 544 -->

[Transcript lines 544-544](../transcript.txt#L544-L544)

```text
redraw bandwidth <10% of a full frame; no new authority path outside named
```

<!-- workflow:excerpt C003 555 556 -->

[Transcript lines 555-556](../transcript.txt#L555-L556)

```text
(fuel-spinning world raises no other world's p99 >10%); F12.11 delta and
binary frames; F12.12 many worlds per node; F12.13 journal replication
```

<!-- workflow:excerpt C003 632 634 -->

[Transcript lines 632-634](../transcript.txt#L632-L634)

```text
6. Said raw keystrokes in alternate mode were `main`'s default. `main` never
   uses the alternate screen (`\x1b[?1049` is absent); it clear-repaints the
   primary screen per view. Alternate screen is an improvement to make.
```

<!-- workflow:excerpt C003 705 708 -->

[Transcript lines 705-708](../transcript.txt#L705-L708)

```text
On a clean LAN both are fine — our frame-based JSON and per-event full
repaints dominate cost anyway. On lossy or high-RTT links QUIC wins. The
interactive latency story is mostly about **render batching** (§5), not the
transport.
```

<!-- workflow:excerpt C003 743 747 -->

[Transcript lines 743-747](../transcript.txt#L743-L747)

```text
- **Tier 2 — xterm, GNOME Terminal, Windows Terminal, macOS Terminal:** no
  kitty keyboard → escape-timeout disambiguation (the guest's `Key.parse`
  heuristics today); cell grid only; **server-side redraw batching** keeps fps
  acceptable; an incremental/diff renderer helps most here (today every view
  is a full `\x1b[H\x1b[2J` repaint).
```

<!-- workflow:excerpt C003 751 752 -->

[Transcript lines 751-752](../transcript.txt#L751-L752)

```text
Every tier benefits from the batching work in §5; the kitty keyboard protocol
would also improve the **raw input** path, not just presentation.
```

## Normalized purpose

Takes the output side: today's per-event full clear-repaint on the primary screen, moving raw sessions to the alternate screen, a diff renderer that sends only changed cells, and server-side redraw batching, measured against the F11.9 baseline and related to F12.11 delta frames.

## Initial thoughts

- Every view is a full `\x1b[H\x1b[2J` repaint on the primary screen (L166-L169; pkgs/id/src/world_presentation.rs:370); `\x1b[?1049` appears nowhere in pkgs/id.
- The alternate screen is a small, separate improvement (L369-L370; #545). L632-L634 corrects the earlier claim that it was main's default.
- Redraw batching (a few ms of coalescing, L343-L344) is server-side and matters more for latency than SSH versus QUIC (L705-L708); #561.
- A diff renderer helps tier 2 most (L745-L747; #562). It is the terminal counterpart of F12.11 delta frames (L555-L556; #536 D1-029), measured against the F11.9 baseline (L518-L519; #542).
- Host events already coalesce through a tokio watch (L162-L165); committed events still send Event plus a full View per session (L159-L161).

## Inferred suggestions

- Inference: Leave the alternate screen on every exit path, including disconnect and a server panic.
- Inference: Synchronized output (DEC 2026) pairs with batching on tier-1 terminals.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
