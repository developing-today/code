# kitty-disambiguate-v1 profile reference

- Status: **implemented (Slice A)** in `pkgs/id/examples/roc-world/Key.roc`
  (`Key.parse_events`, `Key.to_legacy`); no activation.
- Upstream specification:
  [Comprehensive keyboard handling in terminals](https://sw.kovidgoyal.net/kitty/keyboard-protocol/),
  verified by direct fetch on **2026-11-02** (sections Quickstart, Key codes,
  Modifiers, Event types, Progressive enhancement, Disambiguate escape codes,
  Legacy key event encoding, Functional key definitions).
- Design record: `doc/2026-11-01_design_id787.md` §§4.1–4.2, 5–6 (Slice A).
  Activation (negotiation, assembler, browser encoder) is slices B–F and is
  **gated on F13.4 detection (#786, still open)**. This profile must only be
  selected after an explicit opt-in; the push/pop owner is slice E.

## Wire summary (verified against the upstream spec)

- Push `CSI > 1 u` (`ESC [ > 1 u`), pop `CSI < u` (default one entry).
  Query `CSI ? u`. Detection ownership is F13.4's, not this profile.
- Active-stream grammar (complete tokens; framing is slice B):
  plain UTF-8 text runs, legacy bytes `0x0d` (Enter), `0x09` (Tab),
  `0x7f`/`0x08` (Backspace), and
- `CSI number [; modifiers] u` — number is a Unicode codepoint
  (always the unshifted form, e.g. `97` for `a`, never `65`); CSI-u
  functional codes handled here are only `27` Escape, `13` Enter, `9` Tab,
  and `127` Backspace;
  - `CSI 1 [; modifiers] [A B C D E F H P Q S]` letter forms
    (`1` omitted when no modifiers);
  - `CSI number [; modifiers] ~` tilde forms;
  - bare `CSI Z` (Btab terminfo standard) for Shift+Tab.
- Modifiers: shift=1, alt=2, ctrl=4, super=8, hyper=16, meta=32,
  caps_lock=64, num_lock=128; encoded as **1 + bits** (`1` = none).
- Tilde functional numbers: `2` Insert, `3` Delete, `5` PageUp, `6` PageDown,
  `7` Home, `8` End, `11`–`15` F1–F5, `17`–`21` F6–F10, `23`–`24` F11–F12;
  letters `A/B/C/D` arrows, `F` End, `H` Home, `P/Q` F1/F2, `S` F4.
- Enter/Tab/Backspace keep legacy bytes so a crashed session still accepts
  `reset` at a shell prompt (spec Quickstart).

## Non-obvious resolutions (spec-derived, encoded in `parse_events`)

- Bare `ESC` (`CSI 27 u`) is Escape; `Alt+x` is `CSI x-code ; 3 u`.
  A lone trailing `ESC` byte is `Unknown`: active decoding uses no
  Escape timeout, so a bare ESC can never be confirmed as the Escape key.
- `Tab` vs `Ctrl+i`: `0x09` vs `CSI 105 ; 5 u`.
  `Enter` vs `Ctrl+m`: `0x0d` vs `CSI 109 ; 5 u`. Bytes `0x0a` and raw
  C0 controls (`0x00`–`0x1f` except `0x09`/`0x0d`) are `Unknown`: the
  profile never emits them (disambiguated Ctrl arrives as CSI-u).
- Normalization: known US-ASCII shifted printable forms carry Shift consumed
  into the resulting scalar (`CSI 97 ; 2 u` → `A`, no Shift bit; shifted
  punctuation uses its US-ASCII counterpart); Alt/Ctrl bits are preserved.
  For non-ASCII scalars without a profile-supplied shifted alternate, Shift is
  retained rather than guessing a layout-specific result. Legacy text cannot
  prove physical Shift state, so none is invented.
- `CSI Z` and canonical `CSI 9 ; 2 u` both decode to Shift+Tab
  (Btab is universal; the erratum confirms the `9 ; 2` form).
- `CSI R` is `Unknown`: per the spec it collides with the Cursor Position
  Report and was removed from the F3 encodings (F3 is `13 ~` only).
- `CSI 1 ~` / `CSI 4 ~` are `Unknown`: Find/Home and End alternatives
  encodings differ per terminal; never guess.
- `CSI 29 ~` (Menu) is `Unknown`: no named key; deferred with keypad.
- Event types (`:2`/`:3`, including explicit `:1`), alternate keys
  (`:` sub-fields) and associated text (extra `;` fields) are `Unknown`:
  this profile requests none of them.
- Keypad (`KP_*` 57399–57427), media keys (57428–57440), modifier-key
  reports (57441–57454), `F13`–`F35` (57376–57398), caps/scroll/num-lock
  and all other PUA function codes (57344–63743) are `Unknown`: deferred
  to a later profile extension, never guessed as application actions.
  Surrogates and codepoints above `U+10FFFF` are `Unknown`.
- Key number `0` (text-only form), empty fields, and modifier values
  outside `1..=256` are `Unknown`.

## Executable corpus

- `Key.roc` `expect` blocks: the authoritative corpus (13 legacy
  `Key.parse` expectations unchanged, plus rich `parse_events` fixtures
  and `to_legacy` projection checks). Run with pinned
  `bash scripts/roc-test.sh` from `pkgs/id` (`roc-test` just recipe).
- `kitty-disambiguate-v1-fixtures.json` (same directory): the
  language-neutral mirror of that corpus for the slice C (browser
  encoder) and slice D (parity oracle) consumers. Decode with the actual
  Roc `parse_events`, never with a re-implemented decoder (invariant 1).
- Captured-terminal fixtures: none yet; slice B/D must record real
  terminal bytes (terminal/version, negotiated profile, Escape/Alt
  results, reset behavior) before claiming end-to-end support.

## Compatibility projection

`Key.parse` is byte-for-byte unchanged, including the modified-arrow
`Unknown` expectation. `Key.to_legacy` projects rich events back only
when representable (unmodified named/printable keys, Ctrl+letter);
anything with modifiers beyond Ctrl+letter, and all function keys, stays
`Unknown` rather than discarding modifiers to make an old app act.
