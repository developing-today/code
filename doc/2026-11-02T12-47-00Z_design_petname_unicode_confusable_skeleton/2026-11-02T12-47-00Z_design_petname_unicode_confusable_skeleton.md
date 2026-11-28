# Petname anti-mimicry: Unicode UTS #39 confusable skeleton

- **Date:** 2026-11-02T12-47-00Z
- **Kind:** design
- **Issue:** developing-today/code#2899 (follow-ups to #2069 and #1674)
- **Code:** `pkgs/id/crates/id-core/src/petname.rs` (`skeleton()`), PR #4268

## Problem

Petnames are a trust surface: a contact's petname is how the owner recognizes
who they are talking to, so `Petnames::set` refuses a new petname that is
confusable with an existing petname or an alleged name. Until now the
confusability check was a handwritten ASCII skeleton in `skeleton()`:

- `fold()` strips Cf format characters (#1674) and lowercases;
- `skeleton()` then mapped a fixed set of ASCII look-alikes (`0`/`o`,
  `1`/`|`/`i`/`l`, `8`/`b`, ...; the `I`/`8` additions are #2069).

That map cannot enumerate cross-script homoglyphs. Verified on main before
this change: Cyrillic U+0430/U+0410/U+043E/U+0435/U+0421 and Greek U+03BF
pairs (e.g. Latin `PayPal` vs Cyrillic-а `Pаypal`) produced different folds
and different skeletons, so `set()` accepted them — a spoof petname needed
no invisible characters to defeat the guard. `labels()` keys collision
fingerprints by the same skeleton, so legacy cross-script collisions also
went unfingerprinted.

## Decision

`skeleton()` now computes, in order:

1. `fold(name)` — strip Cf format characters, lowercase (unchanged, #1674).
2. The Unicode UTS #39 confusable skeleton of the folded name, via the
   `unicode-security` crate (v0.1.3, `unicode_security::skeleton`).
3. `fold()` again over the skeleton output, so any case/format characters
   the skeleton mapping introduces are normalized the same way as input.
4. The existing petname-specific ASCII look-alike map, unchanged.

Both comparison paths consume this one function — `Petnames::set` (against
existing petnames and alleged names) and `labels()` collision keying — so
legacy cross-script collisions receive fingerprints from the same skeleton
that now refuses new ones.

## Rationale and alternatives

- **UTS #39 skeleton, not a bigger handwritten map.** The confusable table
  is a maintained Unicode data set covering Cyrillic, Greek, and the other
  scripts; a hand-maintained map is what left the cross-script hole after
  two rounds of ASCII additions (#2069, #1674).
- **ASCII map retained, applied after the Unicode skeleton.** UTS #39 does
  not treat leet-style digit/letter pairs (`1`/`l`, `0`/`o`, `8`/`b`,
  `rn`/`m`) as confusable, and existing behavior depends on them
  (`PayPa1`/`rnurn` controls). Dropping the map would regress #2069.
- **Folding order: fold before and after.** Lowercasing first puts capital
  look-alikes into the classes the skeleton data expects; folding the
  skeleton output keeps the pipeline idempotent in case/format handling.
- **Rejected: blanket rejection of non-Latin names.** Petnames legitimately
  exist in other scripts; a single-script Cyrillic name (e.g. `Иван`) must
  stay accepted. The skeleton only makes cross-script _look-alikes of an
  existing name_ collide; unrelated names in any script are unaffected.
- **Rejected: mixed-script rejection alone.** It misses same-name spoofs
  written with homoglyphs and rejects legitimate mixed-script names that
  mimic nothing when used as the sole test.

## Compatibility

- Names previously accepted keep the same skeleton unless they are
  UTS #39-confusable with another name — in which case the pair was a
  spoofing hazard by definition. Single-script non-Latin names and
  unrelated names are unchanged.
- `labels()` fingerprints: pairs that previously shared no skeleton key
  (cross-script collisions already stored) now key together and receive
  distinguishing fingerprints, which is the intended disclosure behavior.

## Verification

Regression tests in `petname.rs` (`test_` prefix per pkgs/id/AGENTS.md):

- six Cyrillic/Greek homoglyph pairs refused against both an existing
  petname and an alleged name;
- ASCII controls (`PayPa1`, `rnurn`, `M0m`) still refused;
- unrelated names and a single-script Cyrillic name accepted;
- legacy cross-script petname collisions get distinct `labels()`
  fingerprints.

Run: `cargo test --lib petname` in `pkgs/id`.

---

## 2026-11-02T12-47-00Z Decision: record added with the change

This record is filed in the same PR as the implementation (PR #4268) per
the pkgs/id documentation protocol; the pipeline above is the policy the
shipped code implements, with no deviations.
