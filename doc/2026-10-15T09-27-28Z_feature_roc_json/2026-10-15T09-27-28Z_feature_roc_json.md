# Pure-Roc JSON for portable world programs

## Intent

P9.4's first slice adds a small JSON library to the import-free Roc world
platform, then uses it in the lounge example for host events, outgoing
capability requests, and records. The module must have no host imports or
package dependency.

## Design boundaries

- JSON values are `Null`, `Bool`, `Number`, `String`, `Array`, and `Object`.
- Number values retain their source decimal lexeme rather than silently
  converting to a machine float. Constructors/encoders therefore preserve
  precision; users who need arithmetic must explicitly parse a number using a
  suitable numeric API.
- Object members are represented as a list of key/value pairs. Accessors
  return explicit `Result` values and do not coerce mismatched types.
- `decode` consumes one complete JSON document; malformed, truncated, or
  trailing input is an error. `encode` escapes strings and emits compact JSON.
- Lounge host-event handling and generated request/record documents use this
  API, while its human-facing view and snapshot format remain unchanged.

## Initial implementation plan

Implement recursive parsing and encoding in `pkgs/id/examples/roc-world/RocJson.roc`,
export the module from `platform.roc`, and add module-level `expect` coverage
for nested values, escaping, malformed input, and round trips. Build the lounge
example with the pinned Roc nightly. Document implementation limits below as
they are discovered.

## Implementation status

Implemented in `pkgs/id/examples/roc-world/RocJson.roc` with a nominal `RocJson`
value type, compact encoder, complete-document decoder, first-match object
lookup, and explicit string/number/bool/array/object accessors. The module is
exposed to apps by the world platform as `RocJson`. Lounge host events are now
decoded before dispatch. Requests and records are built as values and encoded;
the prior player-name sanitizer, capability behavior, records schema, and
checkpoint text layout are retained.

Module `expect` coverage includes nested objects/arrays, all JSON primitives,
quoted/backslash/control-character handling, Unicode surrogate pairs,
malformed and truncated inputs, trailing data, number grammar, typed access,
and encode/decode round trips. The module and value type use `RocJson` to avoid
shadowing Roc's built-in `Json` type in apps importing the platform.

Duplicate object keys are retained and `get` returns the first match. Number
lexemes are preserved rather than converted for arithmetic. No host effects,
platform packages, numeric coercions, or host JSON imports are used. Broader
standard-library helpers and Ratatui adapters remain outside this slice.

---

## 2026-10-15T09-39-59Z Update: bounded decode and checked encode

Hardening adds a 65,536-byte UTF-8 input limit and 64-level container nesting
limit. `decode` returns `InputTooLarge` or `NestingTooDeep` explicitly, with
tests at and one beyond each limit. Encoding has the same depth ceiling and
65,536-byte output ceiling. `RocJson.encode` now returns `Outcome(Str)` and
validates every `Number(Str)` lexeme with the JSON number grammar, returning
`InvalidNumber` rather than emitting invalid JSON. Lounge uses a shared
`encode_or_crash` boundary for ABI functions returning `Str`, so it never
silently replaces an encoding failure with empty or malformed data.

Generic `Outcome` inspection methods support direct accessor expectations
without compile-time unused-branch warnings. Remaining limitations include no arithmetic number conversion, no duplicate
key rejection (lookup continues to use the first occurrence), coarse parse
errors beyond the explicit limit/number cases, and output size is checked only
after the bounded-depth value has been serialized.
