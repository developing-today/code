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

/**
 * Rich (kitty-disambiguate-v1) terminal encoding for world applications —
 * Slice C of issue #787, additive alongside the legacy encoder above.
 *
 * This encoder is NOT used by the UI: nothing calls it yet. It exists so a
 * future per-connection opt-in (Slice F) can select it, exactly as the
 * design requires — `terminalBytesForKey` stays the default legacy path,
 * unchanged. Parity with a terminal is equality of the events the Roc
 * `Key.parse_events` decoder (Slice A) produces, not identical bytes:
 * plain UTF-8 text and a CSI-u form of the same scalar are the same event.
 *
 * Encoding choices, per doc/2026-11-02T_reference_kitty_disambiguate_v1.md:
 *
 * - Escape is always `CSI 27 u` (modified: `CSI 27 ; <mod> u`), never a
 *   bare ESC byte — in the active profile a lone ESC is explicitly
 *   Unknown, so this is what makes Escape versus Alt+<key> unambiguous.
 * - Enter, Tab, and Backspace keep their legacy bytes when unmodified
 *   (`\r`, `\t`, `\x7f`), so a crashed session still accepts `reset`.
 *   With any modifier they use their CSI-u functional codes (13, 9, 127),
 *   which is what distinguishes Tab from Ctrl+i (`CSI 105 ; 5 u`) and
 *   Enter from Ctrl+m (`CSI 109 ; 5 u`). Shift+Tab uses the canonical
 *   `CSI 9 ; 2 u` (the decoder also accepts the `CSI Z` btab form).
 * - Arrows, Home, and End keep their legacy CSI letter bytes unmodified
 *   and carry modifiers as `CSI 1 ; <mod> <letter>`; Delete, Insert,
 *   PageUp, PageDown, and F1–F12 use CSI tilde forms, modified as
 *   `CSI <code> ; <mod> ~`. The legacy encoder does not encode Insert or
 *   the F keys at all; the rich encoder adds them because the profile's
 *   tilde table defines them.
 * - A printable scalar with no modifier, or Shift alone, stays plain
 *   UTF-8 text: Shift is consumed into the resulting scalar (an
 *   uppercase letter, shifted punctuation), as the profile normalizes.
 * - A printable scalar with Ctrl and/or Alt (but not both) is sent as
 *   CSI-u with the scalar's UNSHIFTED codepoint and the full modifier
 *   parameter (e.g. Ctrl+Shift+A is `CSI 97 ; 6 u`, which the decoder
 *   normalizes back to scalar `A` with Control). US-ASCII shifted
 *   punctuation unshifts to its base key (`!` → `1`); for non-ASCII
 *   scalars there is no profile unshifted form, so the scalar's own
 *   codepoint is used and Shift stays in the modifier parameter, which
 *   the decoder likewise retains for non-ASCII.
 * - Ctrl+Alt printable input is AltGr and stays plain UTF-8 text, the
 *   same policy as the legacy path — it is never reinterpreted as a
 *   command. (Ctrl+Alt on a named key is not AltGr text and does carry
 *   both modifiers, e.g. Ctrl+Alt+Delete is `CSI 3 ; 7 ~`.)
 *
 * The modifier parameter is the legacy xterm one (1 + Shift + 2·Alt +
 * 4·Ctrl). Browser events expose no Super/Hyper bit in the
 * `KeyEventLike` shape, and Meta stays excluded below, so no modifier
 * beyond Shift/Alt/Ctrl can be expressed here. Meta/OS shortcuts and
 * Dead/Process/Unidentified keys are excluded, exactly as legacy.
 * Multi-scalar committed text is not a physical key and returns null —
 * it stays on the ordinary text path, as in the legacy encoder.
 */
interface RichSpecialKey {
  plain: string;
  csiU?: number;
  csiLetter?: string;
  csiTilde?: number;
}

const richSpecialKeys: Record<string, RichSpecialKey> = {
  Enter: { plain: "\r", csiU: 13 },
  Tab: { plain: "\t", csiU: 9 },
  Backspace: { plain: "\x7f", csiU: 127 },
  // Unmodified Escape is the CSI-u form itself: a bare ESC byte is
  // Unknown to the profile decoder, never a confirmed Escape press.
  Escape: { plain: "\x1b[27u", csiU: 27 },
  Insert: { plain: "\x1b[2~", csiTilde: 2 },
  Delete: { plain: "\x1b[3~", csiTilde: 3 },
  PageUp: { plain: "\x1b[5~", csiTilde: 5 },
  PageDown: { plain: "\x1b[6~", csiTilde: 6 },
  Home: { plain: "\x1b[H", csiLetter: "H" },
  End: { plain: "\x1b[F", csiLetter: "F" },
  ArrowUp: { plain: "\x1b[A", csiLetter: "A" },
  ArrowDown: { plain: "\x1b[B", csiLetter: "B" },
  ArrowRight: { plain: "\x1b[C", csiLetter: "C" },
  ArrowLeft: { plain: "\x1b[D", csiLetter: "D" },
  F1: { plain: "\x1b[11~", csiTilde: 11 },
  F2: { plain: "\x1b[12~", csiTilde: 12 },
  F3: { plain: "\x1b[13~", csiTilde: 13 },
  F4: { plain: "\x1b[14~", csiTilde: 14 },
  F5: { plain: "\x1b[15~", csiTilde: 15 },
  F6: { plain: "\x1b[17~", csiTilde: 17 },
  F7: { plain: "\x1b[18~", csiTilde: 18 },
  F8: { plain: "\x1b[19~", csiTilde: 19 },
  F9: { plain: "\x1b[20~", csiTilde: 20 },
  F10: { plain: "\x1b[21~", csiTilde: 21 },
  F11: { plain: "\x1b[23~", csiTilde: 23 },
  F12: { plain: "\x1b[24~", csiTilde: 24 },
};

/** US-ASCII shifted punctuation → the unshifted key it sits on. */
const unshiftedAscii: Record<string, string> = {
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

function unshiftedScalar(key: string): string {
  if (/^[A-Z]$/.test(key)) return key.toLowerCase();
  return Object.hasOwn(unshiftedAscii, key) ? unshiftedAscii[key] : key;
}

/**
 * Encode a browser key under the kitty-disambiguate-v1 profile.
 * Returns null for the same exclusions as the legacy encoder, plus
 * anything the profile leaves Unknown (F13+, CapsLock and other
 * modifier-only or unsupported named keys, multi-scalar text).
 * Unused by the UI in this slice (see the block comment above).
 */
export function richTerminalBytesForKey(event: KeyEventLike): Uint8Array | null {
  if (event.metaKey || event.key === "Unidentified" || event.key === "Dead" || event.key === "Process") {
    return null;
  }

  if (Object.hasOwn(Object.prototype, event.key)) return null;

  const encode = (text: string) => new TextEncoder().encode(text);
  const mod = modifierParameter(event);

  const special = Object.hasOwn(richSpecialKeys, event.key) ? richSpecialKeys[event.key] : undefined;
  if (special) {
    if (mod === 1) return encode(special.plain);
    if (special.csiU !== undefined) return encode(`\x1b[${special.csiU};${mod}u`);
    if (special.csiTilde !== undefined) return encode(`\x1b[${special.csiTilde};${mod}~`);
    return encode(`\x1b[1;${mod}${special.csiLetter}`);
  }

  const key = event.key;
  const codePoints = Array.from(key);
  const printable =
    key.length === 1 || codePoints.length === 1 || (codePoints.length > 1 && /[^\p{L}\p{N}]/u.test(key));
  if (!printable || key.length === 0) return null;

  // AltGr (Ctrl+Alt printable) and unmodified / Shift-only text stay
  // plain UTF-8; Shift is already consumed into the scalar.
  if (!event.ctrlKey && !event.altKey) return encode(key);
  if (event.ctrlKey && event.altKey) return encode(key);

  const codePoint = unshiftedScalar(key).codePointAt(0);
  if (codePoint === undefined) return null;
  return encode(`\x1b[${codePoint};${mod}u`);
}
