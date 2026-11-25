# D1 slice 5: display forms

Issue: #13886 (parent epic #13880). Builds on slice 1 (the grammar) and slice 2 (the
`set/` form) in this folder, R1's carrier table (the chat-line and email rows), D3 §5
(supersede chains and tombstones) and D5 (`doc/2026-11-25T02-40-00Z_design_id-apps_d5_link-display/`),
which left "the display form of a URI" to this slice.

Status: proposal. Slice 5 of 5. Slices 3 (failure matrix) and 4 (carriers and
hints) are still open.

Executable reference: `display_reference.py` in this folder
(`python3 -I display_reference.py self-test`, stdlib only). It loads the slice 1 and
slice 2 codecs and prints:

```text
ok: 6 golden labels, 256 URIs delimited, wrapped, extracted and labelled
(77 need brackets), 15 badge cases
```

## 1. The decision in one paragraph

The canonical URI stays the only name, and people see it in three other ways. In
plain text it is written in angle brackets, as RFC 3986 Appendix C suggests, so its
end is never in doubt and a wrapped line still reads back. In a user interface it
is a short label: the shape in words, each digest cut to its first and last eight
hex digits, hints hidden. Next to the label, a resolver's answer adds a few fixed
state words: `edited`, `fork`, `removed`, `stale` and so on. The label and the
words are for recognition only. Copying, opening, comparing and QR codes always use
the full URI.

## 2. Plain text: angle brackets

- **Emitters always write `<id://…>`** in plain text: chat lines, commit messages,
  email bodies and headers, logs meant for people. Markdown renders the same text as
  a link, because CommonMark autolinks accept any scheme.
- **Readers accept the bracketed form across line breaks.** Whitespace inside the
  brackets is removed before parsing, so a URI a mail client or terminal wrapped
  mid-digest reads back whole. A v2 URI never contains whitespace, so nothing real is
  lost.
- **Readers also accept a bare `id://…`.** It ends at whitespace, and trailing prose
  punctuation (`.,;:!?)]}'"`) is dropped. The path of a v2 URI never ends in one of
  those characters, so a bare copy can lose at most the last character of a hint or a
  fragment, never change what is named. The reference checks this on every URI in
  its corpus. Of 256 corpus URIs, 77 end in such a character; all of them still come
  back with the same identity.
- **Nothing is repaired.** A candidate that does not parse after those two steps is
  skipped. A truncated digest is not padded, and a mixed-case one is not lower-cased.

## 3. Interfaces: the short label

| URI                                                   | Label                                                              |
| ----------------------------------------------------- | ------------------------------------------------------------------ |
| `id://any/graph/v2/obj/raw-blake3-256.0001…1f`        | `id:obj 00010203…1c1d1e1f`                                         |
| `…/obj/<cid>/bytes/100-200`                           | `id:obj 00010203…1c1d1e1f bytes 100-200`                           |
| `…/head/<cid>/at/<cid>?via=pkarr:…`                   | `id:head 00010203…1c1d1e1f at e788103e…f433e548`                   |
| `id://home.lan/photos%202026/v2/obj/<cid>#n-3`        | `home.lan/photos 2026 · id:obj 00010203…1c1d1e1f #n-3`             |
| `…/set/head/<cid>/verbs=nest,reply/dir=in/count=4096` | `id:set head 00010203…1c1d1e1f verbs=nest,reply dir=in count=4096` |
| `…/set/obj/<cid>/authors=<2 keys>`                    | `id:set obj 00010203…1c1d1e1f authors=2`                           |

Rules:

- **Digests show 64 bits**, eight hex digits on each side of `…`. That is enough to
  tell two links apart at a glance. It is not enough to stop a determined attacker
  from making a look-alike, so a label is never used to decide that two things are
  the same: tools compare full URIs.
- **The portable authority is hidden.** `any/graph` is the common case and adds
  nothing; any other resolver hint and namespace are shown before a `·`, decoded.
- **Hints are never shown.** They are untrusted and outside identity. A fragment is
  shown, because it says which part of the object is meant.
- **Author keys are counted, not shown.** A key elided to 16 hex digits means nothing
  to a person; D5's `identify` shows the authors' names when the reader asks.
- **A non-default hash profile is named** in front of the digest
  (`william3-256:…`), so a profile change is visible.
- **A label never parses.** It holds a space and an ellipsis, which the parser
  rejects, so a label pasted where a URI is expected fails cleanly. The reference
  checks every corpus label, and that no two corpus URIs share a label.

An application chooses where a label appears: D5 shows the object's own title and
offers the label on `identify`. HTML puts the full URI in `href` and the label in
the text.

## 4. State words (badges)

The words come from the resolver's answer, never from the URI. They have a fixed
order and fixed spelling, so the terminal and web clients agree.

| Word            | Shown on        | When                                                                                   |
| --------------- | --------------- | -------------------------------------------------------------------------------------- |
| `unavailable`   | any             | The coalesced "not authorised or not here" answer. Shown alone, so nothing else leaks. |
| `removed`       | `obj/`, `head/` | The chain's current head is a tombstone (D3 §5).                                       |
| `stale`         | `head/…/at/`    | The pinned head is no longer the current one.                                          |
| `fork`          | `obj/`, `head/` | The chain has more than one current head.                                              |
| `edited`        | `head/`         | The chain has more than one version. Same word as D5's node badge.                     |
| `older version` | `obj/`          | A later version of this exact object exists; the reader can open the `head/` form.     |
| `partial`       | `set/`          | A bound or a resolver cap removed members (slice 2's cuts).                            |
| `claimed times` | `set/`          | A time bound was used, so membership rests on untrusted claims.                        |
| `unknown verbs` | `set/`          | The set names a verb this resolver does not know.                                      |

`obj/` does not say `edited`: an object URI names one version, so the useful fact is
that a newer one exists. The 15 cases in the reference cover each word and the
combinations that can occur together.

## 5. Alternatives weighed

| Alternative                                         | Verdict  | Why                                                                                                |
| --------------------------------------------------- | -------- | -------------------------------------------------------------------------------------------------- |
| Bare URIs in plain text                             | Rejected | R1's chat-line row: linkifiers strip trailing `.`, `)` and `'`, and a wrapped line splits the URI. |
| Quoting with backticks or `"…"`                     | Rejected | Not a standard, not autolinked, and quotes are legal near URIs in prose.                           |
| Showing the full URI everywhere                     | Rejected | 101 characters minimum; people stop reading it, which is worse than a short label they do read.    |
| Shorter elision (4 + 4 hex)                         | Rejected | 32 bits collide in large graphs by chance, not only on purpose.                                    |
| Human names (petnames) instead of digests in labels | Deferred | Needs a naming system; D5's object titles already fill that role in the graph view.                |
| Badges encoded in the URI                           | Rejected | State changes after the link is written. The URI names; the resolver reports.                      |

## 6. What changes elsewhere if this is adopted

- D5's `identify` shows the label and the badge words from §4, and `open` keeps using
  the full URI. D5's `edited` and `fork` node badges keep their spelling.
- Commit messages, chat bots and email carriers that write graph links use the
  bracketed form. Slice 4 adds the per-carrier details (headers, QR on paper,
  `mailto:`).
- The Rust `id://` code gets `display` and `extract` when an implementation slice
  picks this up; nothing changes in parsing.

## 7. Open questions

None needs Drew alone. One is for review: whether `older version` should also be
shown on a D2 snapshot embed, where pointing at an old version is the intent. The
reference leaves that choice to D2's renderer.
