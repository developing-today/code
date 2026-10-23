# Regression cases

Each `<area>.txt` file here is replayed on every test run by
[`tests/regression_replay.rs`](../regression_replay.rs). A fuzz or property
failure is minimized and added here, so a fixed bug cannot return unnoticed.
Testing layers are described in the
[testing reference](../../../../doc/2026-03-29T00-00-00Z_reference_testing_architecture/2026-03-29T00-00-00Z_reference_testing_architecture.md).

## Format

One case per line, three fields separated by TAB characters:

```text
area<TAB>input<TAB>expected
```

- `area` must match the file name: `tuple.txt` holds `tuple` cases.
- `input` uses the area's syntax. For `tuple` it is two elements separated by one
  space. Each element is `s:<escaped>` (a UTF-8 string) or `b:<escaped>` (a byte
  string). For `chainext` it is one `b:<escaped>` element: the bytes given to the
  event decoder in `src/chain.rs`. For `ingest` it is one or more `b:<escaped>`
  elements separated by spaces: the events of one delivery. The check delivers them
  in three orders, with repeats, and requires the same applied state in each.
- `expected` is `ok`: the property must hold for that input.
- Blank lines and lines starting with `#` are skipped.

Escapes: `\\` is a backslash, `\0` is NUL, `\t`, `\n`, `\r`, and `\xHH` is any
byte (exactly two hex digits). A space inside an element must be written `\x20`.
Other characters stand for themselves.

## Adding a case from a failing fuzz run

1. Find the minimal failing input in the failure message. The fixed-seed fuzz
   test ends with `Fail(<reason>, (<first>, <second>))`. Proptest's own
   failures print the same tuple.
2. Spell each element with its tag and escapes. `Text("a\0b")` becomes
   `s:a\0b`, and `Bytes([0, 255])` becomes `b:\0\xff`.
3. Append one TAB-separated line, for example
   `tuple<TAB>s:a\0b s:a<TAB>ok`.
4. Check that the new line fails before the fix and passes after it. Commit it
   with the fix.
5. Add the neighbouring cases too (the same values with one more `\0`, the
   other type tag, or a shared prefix). A regression test has to cover the whole
   class of error, not only the instance that was found.

Do not remove a line when its bug is fixed. The list only grows.
