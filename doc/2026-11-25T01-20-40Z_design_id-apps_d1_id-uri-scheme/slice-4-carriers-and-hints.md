# D1 slice 4: carriers and hints

Issue: #13886 (parent epic #13880). Builds on slices 1, 2 and 5 in this folder, R1's
"URI in the wild" table (§4) and question 6 (hint types and resolver privacy), the
fleet transport design (#13752, for what `broker:` and `pkarr:` point at) and D4
(`doc/2026-11-25T01-29-01Z_design_id-apps_d4_sneakernet-bundles/`).

Status: proposal. Slice 4 of 5. Slice 3 (failure matrix) is still open; it waits on
the #13779 revocation decision points.

Executable reference: `carriers_reference.py` in this folder
(`python3 -I carriers_reference.py self-test`, stdlib only). It loads the slice 1, 2
and 5 references and prints:

```text
ok: 3 accepted, 13 rejects, follow policy for 2 readers, 8 carriers (chat-line 312,
email-body 308, email-header 103, paper 103, qr 101, bundle-index 141, link-object
105, clipboard 118 chars), chat kept 4 of 8 long hints, QR obj v5 / head-at v8
alphanumeric, set scope v8 byte, 256 round trips
```

## 1. The decision in one paragraph

Hints stay what slice 1 made them: ordered, unsigned, droppable and outside
identity. This slice adds three things. Hint kinds become extensible: an unknown
kind is kept and skipped, so a new transport never breaks old parsers. A resolver
follows a hint without asking only when it reaches something the reader already
uses, because following a hint tells its operator that someone wants this name. Each
carrier gets a fixed profile: what it keeps, whether it brackets, and how long it
may be. There is no `mailto:` hint.

## 2. Hint kinds

| Kind      | Value (decoded)                                                          | Points at                                                               |
| --------- | ------------------------------------------------------------------------ | ----------------------------------------------------------------------- |
| `broker:` | A broker name, same syntax as a resolver alias                           | A fleet queue broker the reader may already use (#13752 thread 1).      |
| `pkarr:`  | A 52-character z-base32 Ed25519 key                                      | A signed DNS record for an iroh node or relay (#13752 thread 2).        |
| `bundle:` | A content token, `raw-blake3-256.<64 hex>`                               | A D4 bundle by its bundle ID; the bundle may already be on disk.        |
| `https:`  | `host[:port][/path]`, a DNS name with at least one dot                   | A door, as in the commander pattern. No user info, query or fragment.   |
| other     | Kind `[a-z][a-z0-9-]*`, up to 16, not ending in `-`; any non-empty value | Nothing yet. Kept in the URI, skipped by resolvers that do not know it. |

- **Extensible, not open-ended.** A malformed kind is still `invalid_via_hint`, and a
  malformed value of a known kind is `invalid_via_value`. Only the meaning of a new
  well-formed kind is unknown. This replaces slice 1's closed list. Slice 1's own
  script is unchanged so its pinned corpus stays valid; `carriers_reference.parse` is
  the combined parser.
- **Values keep slice 1's spelling.** They are percent-encoded with upper-case
  escapes, so `https:door.example.org/id` is written `via=https:door.example.org%2Fid`.
- **Never signed.** A hint comes from whoever handed over the URI, and names are
  self-certifying, so a hostile hint can waste a lookup or leak interest but cannot
  substitute content. Signing would not change that and would make hints
  undroppable.
- **Never stored.** D3 link objects, D2 embeds and D4 bundle indexes hold identity
  forms (§4). A hint written into a signed object would rot and would publish where
  its author fetches things.

## 3. Following a hint

A resolver first tries what it already has: its local store, then its own
configured sources. Hints come after, in carrier order. For each one:

| Kind      | Followed without asking when                        | Otherwise |
| --------- | --------------------------------------------------- | --------- |
| `bundle:` | that bundle is already on disk (no network at all)  | skipped   |
| `broker:` | the reader already uses that broker                 | ask       |
| `pkarr:`  | the reader has its own pkarr resolver configured    | ask       |
| `https:`  | the host is on the reader's allow list (whole host) | ask       |
| unknown   | never                                               | skipped   |

"Ask" means the reader decides, once, in the client. A link never makes a client
contact a stranger's server on its own. The allow list matches whole host names, so
`door.example.org.evil.example` is still a stranger. The reference checks the
table for a reader with nothing set up and a reader with everything set up.

## 4. Carrier profiles

| Carrier        | Brackets | Hints   | Fragment | Portable only | Length                                 | Worked example (chars) |
| -------------- | -------- | ------- | -------- | ------------- | -------------------------------------- | ---------------------: |
| `chat-line`    | yes      | kept    | kept     | no            | 400, hints dropped from the end to fit |                    312 |
| `email-body`   | yes      | kept    | kept     | no            | none (slice 5 rejoins wrapped lines)   |                    308 |
| `email-header` | yes      | dropped | dropped  | no            | 900, one unfolded line                 |                    103 |
| `paper`        | yes      | dropped | dropped  | yes           | none                                   |                    103 |
| `qr`           | no       | dropped | dropped  | yes           | QR version 10 at level M               |        101 (version 5) |
| `bundle-index` | no       | dropped | dropped  | no            | none                                   |                    141 |
| `link-object`  | no       | dropped | kept     | no            | none                                   |                    105 |
| `clipboard`    | no       | kept    | kept     | no            | none                                   |                    118 |

Why each row is the way it is:

- **Chat line.** R1 measured about 400 usable bytes. If the hints do not fit, they
  are dropped from the end; the name itself is never cut. If even the bare name
  would not fit, the carrier refuses (`too_long_for_carrier`). The worked example
  carries all four kinds of hint and a fragment. With eight 70-character `https:`
  hints, four survive.
- **Email header.** Headers are copied by every relay and kept in archives, so they
  carry the identity form only. URIs have no whitespace, so folding cannot split
  them, but they stay on one line within RFC 5322's 998-octet limit.
- **Paper and QR.** Anyone holding the paper can read it, and it outlives every
  hint. Only portable forms qualify, because a local alias means nothing to a
  stranger. Object and chain-head forms use the upper-case QR alphanumeric form from
  slice 1: version 5 and version 8 at level M. A set URI holds `=` and `,`, which
  are outside that alphabet, so it uses byte mode; the D4 bundle scope from slice 2
  fits version 8. Past version 10 the carrier refuses.
- **Bundle index, link object.** These are hashed or signed records, so they hold
  identity forms. A link object keeps the fragment, because "this object, scrolled to
  that part" is what an embed means. A bundle index does not need one.
- **Clipboard and share sheet.** Exactly what the person copied, hints included; the
  receiving client applies §3.

Each worked example is read back from the carried text, with slice 5's extractor for
the bracketed ones, and names the same thing as the original.

## 5. `mailto:` and other non-hints

- **No `mailto:` hint kind.** Asking a person for an object by email is a human
  action, not a fetch, and an email address in a URI would be copied into every chat
  line and archive the link passes through. To send a graph link by email, use the
  `email-body` or `email-header` carrier.
- **No capability, ticket or token in a hint.** Slice 1 already rejects every query
  key except `via`, and a hint value is only a place to look. A QR invite that also
  grants access carries the URI and a separate signed ticket (R1 row 18).

## 6. Alternatives weighed

| Alternative                         | Verdict  | Why                                                                                              |
| ----------------------------------- | -------- | ------------------------------------------------------------------------------------------------ |
| Keep the closed kind list           | Rejected | Every new transport would need every parser updated first, for a field that is droppable anyway. |
| Follow every hint automatically     | Rejected | A link could make any client announce its interest to any server the link's author chooses.      |
| Signed hint lists                   | Rejected | Names are self-certifying, so a signature adds no integrity; it would make hints undroppable.    |
| Hints in D3 link objects            | Rejected | They rot, and they publish where the author fetches things.                                      |
| `mailto:` hint kind                 | Rejected | A human action, and a privacy leak in every copy (§5).                                           |
| Truncating a URI to fit a chat line | Rejected | A cut name is a different string; only hints may be dropped.                                     |

## 7. What changes elsewhere if this is adopted

- Slice 1's §5 hint list gains the value rules above, and its closed-list rule
  becomes "unknown kinds are kept and skipped".
- D4's bundle writers and D3's link writers store identity forms; both already do.
- The fleet transport work (#13752) can add hint kinds, such as an iroh ticket,
  without a D1 change.
- The Rust `id://` parser gets the extensible kinds and value rules in its
  implementation slice; resolvers get the §3 policy.

## 8. Open questions

None needs Drew alone. One is for review: whether `bundle:` hints for bundles not
on disk should be "ask" (a client could offer to look for the bundle elsewhere)
rather than "skipped". The reference skips them because the hint alone gives no
place to fetch from.
