/** Terminal-compatible input encoding for world applications. */

type KeyEventLike = Pick<KeyboardEvent, "key" | "ctrlKey" | "altKey" | "metaKey" | "shiftKey">;

/**
 * A special (non-printable) key, described by the forms it can take.
 *
 * - `plain` is the byte sequence an unmodified press sends.
 * - `csiLetter` keys have a CSI form ending in a letter (arrows, Home, End,
 *   and Shift+Tab's `Z`): with a modifier they send `CSI 1 ; <mod> <letter>`.
 * - `csiTilde` keys have a CSI form ending in `~` (Delete, PageUp, PageDown):
 *   with a modifier they send `CSI <code> ; <mod> ~`.
 * - Keys with neither form (Enter, Tab without Shift, Backspace, Escape) have
 *   no CSI encoding for a modifier; see the policy in `terminalBytesForKey`.
 */
interface SpecialKey {
  plain: string;
  csiLetter?: string;
  csiTilde?: number;
}

const specialKeys: Record<string, SpecialKey> = {
  Enter: { plain: "\r" },
  Tab: { plain: "\t", csiLetter: "Z" },
  Backspace: { plain: "\x7f" },
  Escape: { plain: "\x1b" },
  Delete: { plain: "\x1b[3~", csiTilde: 3 },
  Home: { plain: "\x1b[H", csiLetter: "H" },
  End: { plain: "\x1b[F", csiLetter: "F" },
  PageUp: { plain: "\x1b[5~", csiTilde: 5 },
  PageDown: { plain: "\x1b[6~", csiTilde: 6 },
  ArrowUp: { plain: "\x1b[A", csiLetter: "A" },
  ArrowDown: { plain: "\x1b[B", csiLetter: "B" },
  ArrowRight: { plain: "\x1b[C", csiLetter: "C" },
  ArrowLeft: { plain: "\x1b[D", csiLetter: "D" },
};

/** xterm modifier parameter: 1 + (Shift ? 1) + (Alt ? 2) + (Ctrl ? 4). */
function modifierParameter(event: KeyEventLike): number {
  return 1 + (event.shiftKey ? 1 : 0) + (event.altKey ? 2 : 0) + (event.ctrlKey ? 4 : 0);
}

/**
 * Encode a browser key as the bytes a terminal sends for that key.
 * Returns null for browser/OS shortcuts and keys the Roc `Key` module does not
 * recognize yet. Printable input is UTF-8; control-letter input is C0.
 *
 * Modified special keys follow one policy (issue #732, Option 1 — encode the
 * modifier; no modified special key ever returns the plain key's bytes):
 *
 * - A key with a CSI form carries the standard xterm modifier parameter:
 *   Delete is `CSI 3 ; <mod> ~` (Ctrl+Delete = `ESC [ 3 ; 5 ~`), arrows,
 *   Home, and End are `CSI 1 ; <mod> <letter>` (Alt+Up = `ESC [ 1 ; 3 A`).
 *   Shift+Tab is the traditional backtab `ESC [ Z`; Shift+Tab with further
 *   modifiers is `CSI 1 ; <mod> Z`, like the other CSI letter keys.
 * - A key with no CSI form (Backspace, Enter, Escape, Tab without Shift) is
 *   ESC-prefixed when modified, the same convention the printable path below
 *   already uses for Alt+printable (Alt+Backspace = `ESC DEL`). The prefix
 *   signals "modified" even though these keys cannot distinguish which
 *   modifier was held.
 *
 * Unmodified special keys, control letters, and AltGr (Ctrl+Alt printable)
 * behaviour are unchanged.
 */
export function terminalBytesForKey(event: KeyEventLike): Uint8Array | null {
  if (event.metaKey || event.key === "Unidentified" || event.key === "Dead" || event.key === "Process") {
    return null;
  }

  if (Object.hasOwn(Object.prototype, event.key)) return null;

  const special = Object.hasOwn(specialKeys, event.key) ? specialKeys[event.key] : undefined;
  if (special) {
    const modified = event.ctrlKey || event.altKey || event.shiftKey;
    if (!modified) return new TextEncoder().encode(special.plain);
    const mod = modifierParameter(event);
    // Tab's CSI form exists only with Shift (backtab); a Tab modified only by
    // Ctrl and/or Alt has no CSI form and falls through to the ESC prefix.
    const hasCsiForm =
      special.csiTilde !== undefined || (special.csiLetter !== undefined && !(event.key === "Tab" && !event.shiftKey));
    if (hasCsiForm && special.csiTilde !== undefined) {
      return new TextEncoder().encode(`\x1b[${special.csiTilde};${mod}~`);
    }
    if (hasCsiForm && special.csiLetter !== undefined) {
      // Plain Shift+Tab keeps its traditional form, with no parameter.
      if (event.key === "Tab" && mod === 2) return new TextEncoder().encode("\x1b[Z");
      return new TextEncoder().encode(`\x1b[1;${mod}${special.csiLetter}`);
    }
    const plain = new TextEncoder().encode(special.plain);
    const withEscape = new Uint8Array(plain.length + 1);
    withEscape[0] = 0x1b;
    withEscape.set(plain, 1);
    return withEscape;
  }

  if (event.ctrlKey && !event.altKey) {
    if (event.key.length !== 1 || !/[a-z]/iu.test(event.key)) return null;
    return Uint8Array.of(event.key.toLowerCase().charCodeAt(0) - 96);
  }

  const key = event.key;
  const codePoints = Array.from(key);
  const printable =
    key.length === 1 || codePoints.length === 1 || (codePoints.length > 1 && /[^\p{L}\p{N}]/u.test(key));
  if (!printable || key.length === 0) return null;
  const text = new TextEncoder().encode(key);
  if (!event.altKey || event.ctrlKey) return text;
  const withEscape = new Uint8Array(text.length + 1);
  withEscape[0] = 0x1b;
  withEscape.set(text, 1);
  return withEscape;
}

// ---------------------------------------------------------------------------
// Slice C of issue #787: the additive rich (kitty) browser encoder.
//
// `kittyBytesForKey` is the browser counterpart of the Roc rich decoder
// `Key.parse_events` (kitty-disambiguate-v1 profile, Slice A). It is NOT
// wired into the UI: `world.ts` keeps using the legacy
// `terminalBytesForKey` above until Slice F selects this encoder through
// an explicit per-connection keyboard option. The legacy encoder and its
// behaviour are unchanged.
//
// The encoding goal is parity of *decoded events*, not byte identity with
// a terminal: every byte string produced here decodes under
// `Key.parse_events` to the logical key event the browser reported.
//
// - Unmodified keys keep the legacy plain bytes, which already decode to
//   the same events — except Escape: a bare ESC byte is `EventUnknown`
//   for the active decoder (it never guesses with a timeout), so Escape
//   is sent as CSI-u `ESC [ 27 u`, which decodes to `NamedEscape`.
// - Any modified key carries its modifiers explicitly, in the standard
//   xterm modifier parameter (see `modifierParameter`):
//   - Escape, Tab, Enter, and Backspace use CSI-u with their kitty key
//     codes (27, 9, 13, 127). This is what makes Escape vs Alt and
//     Tab vs Ctrl+i / Enter vs Ctrl+m unambiguous: Ctrl+i is
//     `ESC [ 105 ; 5 u` (the letter), Ctrl+Tab is `ESC [ 9 ; 5 u`.
//   - CSI-letter keys (arrows, Home, End) keep `CSI 1 ; <mod> <letter>`.
//   - CSI-tilde keys (Delete, PageUp, PageDown) keep `CSI <code> ; <mod> ~`.
//   - Shift+Tab is CSI-u `ESC [ 9 ; <mod> u`, not the `CSI Z` family: the
//     rich decoder accepts plain `ESC [ Z` (Shift only) but rejects a
//     parameterised `CSI 1 ; <mod> Z` as Unknown.
// - Modified printable keys use CSI-u with the *unshifted base* scalar as
//   the key code (lowercase letter, or the digit/punctuation a shifted
//   symbol sits on), mirroring what a kitty terminal sends. The decoder
//   re-applies Shift to the scalar and drops Shift from the modifier set,
//   so Ctrl+Shift+i (browser key `I`) sent as `ESC [ 105 ; 6 u` decodes
//   to scalar `I` with [Control] — the same event a terminal produces.
//   Shift alone is not encoded this way: the browser key already IS the
//   shifted scalar (`A`, `!`), and plain text decodes to exactly that
//   scalar with Shift consumed, per the design's normalization policy.
// - The decoder's Shift re-application is a US-layout table, so it is
//   only trusted when it reconstructs the browser-reported scalar
//   exactly: a Caps Lock uppercase letter arrives with no Shift flag
//   (its case is preserved as-is), and on layouts where Shift produces
//   a scalar the US table does not predict (e.g. AZERTY Shift+& reports
//   `1`), the reported scalar is sent with Shift dropped from the
//   modifier parameter instead of letting the decoder remap it to the
//   US shifted symbol.
// - AltGr (Ctrl+Alt producing a non-letter printable, e.g. `€`) stays
//   plain text, exactly as in the legacy encoder: the browser key is the
//   composed character, not a command chord. Ctrl+Alt with an ASCII
//   letter IS a command chord (AltGr letters arrive as the composed
//   character, not the letter) and uses CSI-u.
// - Meta/OS shortcuts and Dead/Process/Unidentified keys stay excluded,
//   and a modified key that is not a single scalar (multi-scalar
//   committed text) is not fabricated into a physical key: it returns
//   null instead of guessing.
// ---------------------------------------------------------------------------

/** Kitty CSI-u key codes for the named keys that have no CSI letter/tilde form. */
const kittyNamedCodes: Record<string, number> = {
  Escape: 27,
  Tab: 9,
  Enter: 13,
  Backspace: 127,
};

/**
 * The unshifted base key for a shifted symbol: the digit/punctuation a
 * kitty terminal reports as the key code while Shift appears only in the
 * modifier parameter. This is the inverse of the table the Roc decoder
 * uses to re-apply Shift (`normalize_shift` in Key.roc).
 */
const shiftedSymbolBases: Record<string, string> = {
  "!": "1",
  "@": "2",
  "#": "3",
  $: "4",
  "%": "5",
  "^": "6",
  "&": "7",
  "*": "8",
  "(": "9",
  ")": "0",
  _: "-",
  "+": "=",
  "{": "[",
  "}": "]",
  "|": "\\",
  ":": ";",
  '"': "'",
  "<": ",",
  ">": ".",
  "?": "/",
  "~": "`",
};

/** The scalar a kitty terminal would name for this key event's key. */
function kittyBaseScalar(key: string, shiftKey: boolean): string {
  // Lowercasing a letter is only safe when Shift is in the chord: the
  // decoder re-applies Shift to recover the uppercase scalar. Without
  // Shift (Caps Lock, or an Alt chord on an uppercase letter) the
  // reported case IS the scalar and must be preserved as-is.
  if (shiftKey && /^[a-z]$/iu.test(key)) return key.toLowerCase();
  if (shiftKey && Object.hasOwn(shiftedSymbolBases, key)) return shiftedSymbolBases[key];
  return key;
}

/**
 * What the Roc decoder's `normalize_shift` produces for a base scalar
 * when Shift is in the modifier set: letters uppercase, and single-byte
 * digits/punctuation map through the US shift table (the forward
 * direction of `shiftedSymbolBases`). The decoder applies this only to
 * single-byte scalars; anything else passes through unchanged.
 */
function shiftNormalizedScalar(base: string): string {
  if (new TextEncoder().encode(base).length !== 1) return base;
  if (/^[a-z]$/.test(base)) return base.toUpperCase();
  for (const [shifted, unshifted] of Object.entries(shiftedSymbolBases)) {
    if (unshifted === base) return shifted;
  }
  return base;
}

/**
 * Encode a browser key as kitty-disambiguate-v1 bytes that
 * `Key.parse_events` decodes to the same logical event. Returns null for
 * the same exclusions as the legacy encoder, and for modified input that
 * is not a single scalar or a known special key. Inert in this slice:
 * nothing calls this yet (see the Slice C note above).
 */
export function kittyBytesForKey(event: KeyEventLike): Uint8Array | null {
  if (event.metaKey || event.key === "Unidentified" || event.key === "Dead" || event.key === "Process") {
    return null;
  }

  if (Object.hasOwn(Object.prototype, event.key)) return null;

  const encode = (text: string) => new TextEncoder().encode(text);
  const modified = event.ctrlKey || event.altKey || event.shiftKey;

  const special = Object.hasOwn(specialKeys, event.key) ? specialKeys[event.key] : undefined;
  if (special) {
    if (!modified) {
      // Bare ESC is ambiguous/unknown to the active decoder; every other
      // unmodified special's legacy bytes already decode to its event.
      return event.key === "Escape" ? encode("\x1b[27u") : encode(special.plain);
    }
    const mod = modifierParameter(event);
    const namedCode = Object.hasOwn(kittyNamedCodes, event.key) ? kittyNamedCodes[event.key] : undefined;
    if (namedCode !== undefined) return encode(`\x1b[${namedCode};${mod}u`);
    if (special.csiTilde !== undefined) return encode(`\x1b[${special.csiTilde};${mod}~`);
    if (special.csiLetter !== undefined) return encode(`\x1b[1;${mod}${special.csiLetter}`);
    return null;
  }

  const key = event.key;
  const codePoints = Array.from(key);
  const printable =
    key.length === 1 || codePoints.length === 1 || (codePoints.length > 1 && /[^\p{L}\p{N}]/u.test(key));
  if (!printable || key.length === 0) return null;

  // Unmodified and Shift-only printable input is already the resulting
  // scalar; plain UTF-8 decodes to that scalar with Shift consumed.
  if (!event.ctrlKey && !event.altKey) return encode(key);

  // AltGr: Ctrl+Alt whose browser key is a composed non-letter character
  // is text input, not a command chord — preserve it as plain text.
  if (event.ctrlKey && event.altKey && !/^[a-z]$/iu.test(key)) return encode(key);

  // Modified single-scalar input: CSI-u over the unshifted base scalar.
  if (codePoints.length !== 1) return null;
  const base = kittyBaseScalar(key, event.shiftKey);
  const code = base.codePointAt(0);
  if (code === undefined || code < 32) return null;
  if (event.shiftKey && shiftNormalizedScalar(base) !== key) {
    // Shift is in the chord but the decoder's US-table re-application
    // would not reconstruct the reported scalar (Caps Lock + Shift
    // reversing a letter to lowercase, or a non-US layout where Shift
    // produces e.g. a digit): send the reported scalar itself with
    // Shift dropped from the modifier parameter, so the decoder passes
    // it through unchanged instead of remapping it.
    const direct = key.codePointAt(0);
    if (direct === undefined || direct < 32) return null;
    const withoutShift = 1 + (event.altKey ? 2 : 0) + (event.ctrlKey ? 4 : 0);
    return encode(`\x1b[${direct};${withoutShift}u`);
  }
  return encode(`\x1b[${code};${modifierParameter(event)}u`);
}
