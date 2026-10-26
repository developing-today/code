# C004: Presentation capability tiers and the kitty keyboard protocol

[Main reference](../main.md#c004)

## Relevant transcript excerpts

<!-- workflow:excerpt C004 107 110 -->

[Transcript lines 107-110](../transcript.txt#L107-L110)

```text
- **`platform.roc`** requires `layout : model, U64, U64 -> Str` (guest-aware
  reflow). `Key.parse` decodes raw terminal input bytes into a `Key` tag union
  (`Char/Ctrl/Enter/Tab/Backspace/Escape/Delete/arrows/Home/End/PageUp/
  PageDown/Unknown`) matching the browser encoder's sequences.
```

<!-- workflow:excerpt C004 333 333 -->

[Transcript lines 333-333](../transcript.txt#L333-L333)

```text
| Client capability negotiation | yes | yes | client tells the server what it supports |
```

<!-- workflow:excerpt C004 393 393 -->

[Transcript lines 393-393](../transcript.txt#L393-L393)

```text
| Accessibility / slow links | Line | Transcript output works where a 60 fps grid does not |
```

<!-- workflow:excerpt C004 728 752 -->

[Transcript lines 728-752](../transcript.txt#L728-L752)

```text
## 14. Presentation capability tiers (ghostty-class default, graceful degradation)

Agreed target: **ghostty-class terminals as the default supported
presentation** (kitty keyboard protocol, graphics, synchronized output), with
a **degraded experience** when those are absent or the frame rate cannot be
met.

- **Detection is in-band**, so it works identically over SSH and Iroh:
  capability negotiation via escape sequences (`XTGETTCAP`, `CSI ? u`,
  `DECRQM`) from the terminal. The "client capability negotiation" trigger
  (§5) applies to presentation the same way it does to input.
- **Tier 1 — ghostty, kitty, wezterm, …:** kitty keyboard protocol →
  unambiguous Escape vs Alt, key events beyond the legacy byte stream;
  kitty graphics or sixel → images/sprites; synchronized output (DEC 2026)
  → flicker-free repaints; high fps feasible.
- **Tier 2 — xterm, GNOME Terminal, Windows Terminal, macOS Terminal:** no
  kitty keyboard → escape-timeout disambiguation (the guest's `Key.parse`
  heuristics today); cell grid only; **server-side redraw batching** keeps fps
  acceptable; an incremental/diff renderer helps most here (today every view
  is a full `\x1b[H\x1b[2J` repaint).
- **Tier 3 — dumb or slow:** the line mode (§5-§7) *is* the degraded
  presentation — transcript output, no grid.

Every tier benefits from the batching work in §5; the kitty keyboard protocol
would also improve the **raw input** path, not just presentation.
```

<!-- workflow:excerpt C004 792 793 -->

[Transcript lines 792-793](../transcript.txt#L792-L793)

```text
6. **Presentation tier detection** — do the kitty-protocol probe and redraw
   batching land with #211's reconciliation, or as a separate follow-up?
```

## Normalized purpose

Takes the agreed ghostty-class default presentation with graceful degradation: in-band terminal capability detection, three tiers (kitty keyboard, graphics and synchronized output; cell grid with batching; line mode), and the kitty keyboard protocol on the raw input path.

## Initial thoughts

- Agreed target: ghostty-class terminals (kitty keyboard, graphics, synchronized output) are the default presentation, with a degraded experience otherwise (L730-L733).
- Detection is in-band (XTGETTCAP, `CSI ? u`, DECRQM), so it works the same over SSH and iroh (L735-L738); nothing in pkgs/id probes a terminal today.
- Tier 3 is the line mode (L748-L749); tier 2 relies on batching and the diff renderer (L743-L747).
- The kitty keyboard protocol also improves raw input (L751-L752); today Key.parse disambiguates escapes by heuristics (L107-L110, L744-L745).
- Q6 asked whether the probe lands with #211's reconciliation or as a follow-up (L792-L793); #211 has merged, so only the follow-up remains. #535 tracks D1-052 and D1-053.

## Inferred suggestions

- Inference: An in-band probe over SSH costs a round trip; probe once with a timeout and cache the tier per session.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
