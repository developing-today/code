# Design — the `id://` URI scheme and per-namespace handlers/encodings (#9117)

Status: design record (no code changed by this record). Issue:
[developing-today/code#9117](https://github.com/developing-today/code/issues/9117);
parent epic [developing-today/code#9107](https://github.com/developing-today/code/issues/9107);
program #9106 (extensible primitives / federation / namespaces), §4
lines 46–56. Statement inventory: S4.1–S4.18 in
`doc/2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md`.
Recorded 2026-11-17 by queue task 20311 (lane any-c).

## 1. What this settles

The form of an `id://` URL: its grammar against RFC 3986, what each part
names, how a namespace declares its handler and encoding, and how a URL
selects them. It explicitly evaluates Drew's `idb64+...` sketch and
supersedes it (§5). The array-address core that namespace paths encode
is #9118's design; this record settles the seam between the two (the
encoding declaration, §4), not the core's byte format.

## 2. Grammar

```abnf
id-URI      = "id://" authority path-abempty [ "?" query ]
authority   = host [ ":" port ]              ; RFC 3986 §3.2, no userinfo
path-abempty = "/" namespace *( "/" segment )
namespace   = ALPHA *( ALPHA / DIGIT / "-" ) ; lowercase after normalisation
segment     = *pchar                         ; RFC 3986 §3.3
```

- The **authority is the server** (S4.1, S4.2): the host whose namespace
  registry interprets the URL. `userinfo` is forbidden — a resolver must
  reject an `id://` URL carrying one; credentials never ride URLs.
- The **first path segment is the namespace name** within that server's
  registry (S4.3). The remaining segments are the namespace path, whose
  meaning belongs entirely to the namespace (§4).
- **Fragment** (`#...`): permitted syntactically, never sent to a
  resolver, stripped before resolution. It is a client-side locator
  only. (Chosen default; open question Q1.)
- **Query**: permitted; its parameters are namespace-declared (§4). A
  resolver rejects undeclared query parameters rather than ignoring
  them, so a typo'd parameter fails loudly instead of resolving to a
  silently different object. (Chosen default; Q2.)

## 3. Normalisation and comparison

Per RFC 3986 §6, with the choices S4.17 requires made explicit:

- Scheme and namespace name are case-insensitive and normalised to
  lowercase. The host is normalised per DNS rules (lowercase; IDNA
  names are stored and compared as punycode).
- **Namespace-path segments are case-preserving by default.** Whether a
  namespace's paths are case-sensitive is a property of its declared
  core type (§4); the URL layer never folds path case on its own.
- Percent-encoded unreserved characters are decoded at parse time
  (RFC 3986 §6.2.2.2); hex digits in percent-encodings are uppercase
  in canonical form. Under the `text` encoding (§4), percent-decoded
  segments must be valid UTF-8; invalid UTF-8 is a parse error, never
  a lossy substitution.
- A trailing slash is significant: `.../a/` and `.../a` are different
  namespace paths (an empty final segment is a segment, not an
  absence). (Chosen default; Q3.)
- **Length**: a URL longer than 8 KiB is rejected at parse with a
  named error — never truncated. A namespace may declare a lower
  limit. (Chosen default; Q4.)

## 4. Namespace declarations: handler and encoding

A namespace definition (S4.18) declares at least:

| Field          | Meaning                                                                 |
| -------------- | ----------------------------------------------------------------------- |
| `name`         | The namespace segment (§2), unique within the server's registry.        |
| `core_type`    | The addressing core the namespace path encodes (e.g. the array-address core of #9118, a plain hierarchical path core, a hash core). |
| `handler`      | A handler identifier from the server's handler registry — the code that resolves and writes paths in this namespace. |
| `encoding`     | How core addresses map to/from path segments: `text` or `b64`, or a registry extension (§6). |
| `rules`        | Resolution/write rules: who may resolve, who may write, case sensitivity of paths. |

Encodings:

- **`text`** (the default, and the common case): namespace-path
  segments are human-readable UTF-8 text, percent-encoded per RFC 3986
  where a character is not a `pchar`. An address item containing `/`
  carries it percent-encoded (`%2F`) — segment structure is never
  ambiguous. This is the case S4.13/S4.16 protect: a person can read
  the URL and know what it names.
- **`b64`**: each segment is base64url (RFC 4648 §5, no padding) of the
  corresponding canonical address bytes of the core type — for
  namespaces whose addresses are binary (e.g. raw hash or array-address
  items that are not text). base64url's alphabet contains no `/`, so
  segment structure survives.

**Selection**: parsing an `id://` URL selects the server by authority,
then the namespace by first segment **in that server's registry**
(S4.3–S4.5), then that namespace's declared `handler` + `encoding`
interpret the rest. An unknown namespace on that server is a
resolution error — never a guess, and never a fallback to another
server's registry: the same URL text resolving differently on
different servers is by design (S4.6, S4.7), and a URL carries no
claim about any server but its authority.

## 5. The `idb64+...` sketch: evaluated, superseded

Drew's sketch put the encoding in a scheme-like prefix (`idb64+...`).
It is **not adopted** (S4.15), for three reasons:

1. **Encoding is per-namespace, not per-URL-scheme** (S4.9, S4.10). A
   scheme-position marker would need one scheme per encoding, and a
   single URL still could not serve two namespaces with different
   encodings — the marker answers the wrong question. The namespace
   segment already names exactly the thing the encoding is a property
   of.
2. **It makes the common case an opaque base64 blob**, which S4.13 and
   S4.16 rule out: the URLs people paste, read, and debug daily would
   be the unreadable ones, and the readable form would be the special
   case.
3. **`+`-suffixed schemes are not RFC 3986 grammar for this** — a
   scheme is a single token; `idb64+foo` parses as the scheme `idb64+foo`
   only if `+` is admitted, minting a new scheme per combination and
   breaking generic URI tooling that dispatches on scheme.

What the sketch wanted — binary addresses must be expressible — is
preserved: it is the `b64` encoding, declared by the namespaces that
need it (§4).

## 6. Expansion without breakage (S4.14)

- New encodings and handlers are **registry entries**, not grammar
  changes: an encoding identifier the parser does not know is a
  resolution error on that server, and adding one never changes how
  existing URLs parse.
- New URL-level features ride the **query** under namespace-declared
  parameters (§2), never new path punctuation.
- The path grammar in §2 is frozen for the `id` scheme: any future
  change that would alter how an existing URL parses is a new scheme,
  not a revision of this one.

## 7. Open questions, with chosen defaults

- **Q1 — fragment semantics.** Chosen: permitted, client-side only,
  stripped at resolution (§2). Rejected: forbidding fragments
  (breaks ordinary URI tooling expectations for no gain).
- **Q2 — undeclared query parameters.** Chosen: reject (§2).
  Rejected: ignore (silent divergence of meaning).
- **Q3 — trailing slash.** Chosen: significant (§3). Rejected:
  normalising it away (collapses two distinct paths in cores where an
  empty final item is meaningful — cf. the array-address core's
  meaningful null/empty slots, #9118).
- **Q4 — length limit.** Chosen: 8 KiB default cap, named error,
  namespace may lower it (§3). Rejected: no cap (unbounded parser
  input) and 2 KiB (too close to long legitimate text paths).
- **Q5 — self-certifying authorities** (a server named by identity
  fingerprint instead of DNS). Deferred to the rendezvous/identity
  designs that own server identity; the authority grammar here is
  RFC 3986 `host`, and a fingerprint form, if adopted there, enters as
  a `host` production those designs define — not as a second authority
  syntax in this record.

## 8. Implementation split (filed as issues, linked from #9117)

1. `id-core`: an `id://` parser/serializer implementing §2–§3
   (grammar, normalisation, length cap, named parse errors) with the
   record's examples as test vectors.
2. Server side: the namespace registry declaration type (§4's fields)
   and URL→namespace selection, including the unknown-namespace and
   undeclared-query-parameter errors.
