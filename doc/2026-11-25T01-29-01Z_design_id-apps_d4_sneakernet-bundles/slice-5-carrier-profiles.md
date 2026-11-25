# D4 slice 5: carrier profiles for sneakernet bundles

Refs #13889 (D4, acceptance item 5). Builds on slice 1
(`2026-11-25T01-29-01Z_design_id-apps_d4_sneakernet-bundles.md`) and on the
D1 slice 4 carrier rules for links
(`../2026-11-25T01-20-40Z_design_id-apps_d1_id-uri-scheme/slice-4-carriers-and-hints.md`).

Reference: `carrier_profiles_reference.py` (stdlib only; it loads
`bundle_reference.py` beside it). `python3 -I carrier_profiles_reference.py`
runs the self-test, `--table` prints every ceiling, and `--cross-check` checks
the QR table and symbol versions against the `segno` package when it is
installed.

## 1. The rule every profile follows

A carrier only tightens slice 1's ceilings and says how the bytes ride. The
bundle bytes never change. Whatever the carrier, the receiver gets back
exactly the slice-1 file and verifies it with slice 1's reader. Nothing the
carrier adds (a file name, an email subject, a printed caption) is signed or
trusted.

## 2. Profiles

| Profile           | Largest bundle      | Shape on the carrier                     | Readable by                    |
| ----------------- | ------------------- | ---------------------------------------- | ------------------------------ |
| `removable-media` | 4,294,967,295 bytes | one bundle per `*.idbundle` file         | whoever holds the stick        |
| `email`           | 7,259,799 bytes     | one bundle per attachment, base64 MIME   | the mail providers on the path |
| `qr-page`         | 2,257 bytes         | one QR code, level M, version 40 at most | anyone who sees the code       |
| `qr-small`        | 964 bytes           | one QR code, level M, version 25 at most | anyone who sees the code       |

Slice 1's format ceiling is about 68.7 GB (1 MiB manifest, 4096 objects of
16 MiB). Every profile here is tighter.

### 2.1 Removable media (the primary case)

- One bundle per file, extension `.idbundle`. A writer puts them in an
  `id-bundles/` directory at the root of the medium. The file name is
  advisory: the first 16 hex digits of the bundle ID. A reader ignores the
  name and computes the ID from the manifest.
- Largest file 2³² − 1 bytes, the FAT32 file limit. FAT32 is the format most
  sticks and cards still ship with, and it is the one every desktop can
  write. A larger set is split (§3).
- A reader never writes to the medium and never runs anything from it. Read
  only media (a write-protected card, a disc) work unchanged.
- Many bundles may share one medium. Each verifies alone.

### 2.2 Email attachment

- One bundle per message, as an attachment named `*.idbundle`, content type
  `application/octet-stream`, base64 transfer encoding. The receiver takes
  the one attachment with that extension and refuses a message with none or
  with several (`email_needs_one_bundle`). The subject and body are free
  text for the people involved and are never trusted.
- Budget: 10,000,000 bytes for the whole message, with 64 KiB kept for the
  headers, the note and the MIME framing. Base64 in 76-character lines with
  CRLF turns that into a raw ceiling of 7,259,799 bytes. The self-test checks
  the size formula against the bytes Python's own MIME writer produces.
- The budget is a design choice, not a provider fact. A sender cannot see
  the receiving server's limit, so the profile aims low. It is one constant
  in the reference.
- This also caps one object at the same size. A larger media object stays
  out of band through its D3 media body, as slice 1 already allows.

### 2.3 QR code

- Text form: `IDB1:` followed by the bundle bytes in base45 (RFC 9285).
  Base45's 45 characters are exactly QR's alphanumeric set, so the whole code
  is one alphanumeric segment. The prefix tells a scanner this is a bundle
  and not a D1 link, whose QR form starts `ID://`.
- Why not raw bytes: most scanning apps hand back text decoded with a guessed
  character set, so arbitrary bytes do not reliably survive. Base45 was made
  for this case and costs about 3% over raw bytes in a QR code (16.5 bits per
  2 bytes).
- Level M, the same as D1's link codes. The decoder is strict: each byte
  string has one encoding, and anything else is `qr_bad_base45`. A text
  without the prefix is `qr_not_a_bundle`. After that, slice 1's own
  rejection codes come through unchanged.
- Two sizes. `qr-page` allows version 40, which is 92.5 mm a side at 0.5 mm
  modules with the quiet zone. `qr-small` allows version 25, which is
  62.5 mm a side. Nobody has tried scanning a version 40 bundle code with
  phones for this design yet. That is why the smaller size exists, and why
  both ceilings are below.

## 3. Splitting a set that does not fit

A writer splits a larger set into parts. Each part is a whole signed bundle
with the same creator, times and scope. Objects are packed in digest order,
filling each part until the next object would not fit. Two writers holding the
same set therefore make the same parts. No part depends on another:
slice 1 §6 already allows dangling links. Slice 3's import report lists what
is still missing, and slice 4's merge fills it in as the other parts arrive.

One object that cannot fit on its own is refused for that carrier with
`object_too_big_for_carrier`. It goes by another carrier, or out of band
through its media body.

## 4. The QR object ceiling, stated honestly

These counts come from real signed bundles that the reference builds, encodes
and reads back through the QR form. They are not estimates. The bundles carry
only the required fields plus the objects. Chat objects have the same shape as
slice 1's example messages (145 bytes). Link objects have the same shape as
its reply link (407 bytes, mostly two 64-character hex digests).

| Code                    | Chat objects | Link objects | With a 96-byte creator binding |
| ----------------------- | ------------ | ------------ | ------------------------------ |
| `qr-page` (version 40)  | 10           | 4            | 9 chat, 4 link                 |
| `qr-small` (version 25) | 3            | 1            | 3 chat, 1 link                 |

What this means:

- One printed code carries a short exchange of about ten messages, or a few
  links. That matches #13889's "a manifest and a handful of objects" at the
  page size, and only barely at the small size.
- Each object costs 57 bytes of manifest entry on top of its own bytes. A
  bundle costs 175 bytes before its first object: header, signature,
  key, identity and times.
- Slice 1's example bundle (2,835 bytes, including a 1,280-byte blob) does not
  fit one code. It splits into two page codes. Without its blob it fits one.
  The small code refuses the blob.
- Slice 3 made a creator binding mandatory. The 96-byte column was a guess
  at its floor, and it was too low: a carried keyset chain is 554 bytes or
  more. So a QR export names the chain instead (65 bytes, close to the
  96-byte column). `slice-3-import-and-binding.md` §8 has the real numbers.
- The object bytes are stand-ins until D3 fixes its byte encoding (slice 1
  §3.3). The ceiling moves with object size, and `--table` prints it per size.

## 5. Privacy per carrier

Slice 1 bundles are signed but not encrypted. A carrier that other people
handle exposes the contents to them: the mail providers for email, anyone who
sees a printed code. The profile table says who can read each one. Sealing a
bundle to a recipient belongs with the grants question in slice 2, which waits
on Drew's bearer versus recipient-bound decision. This slice does not settle
it. Until then, an exporter shows the person who can read the carrier before
it writes.

## 6. Refusal codes added by this slice

| Code                         | Cause                                       |
| ---------------------------- | ------------------------------------------- |
| `qr_not_a_bundle`            | QR text without the `IDB1:` prefix          |
| `qr_bad_base45`              | not the one valid base45 encoding           |
| `email_needs_one_bundle`     | no `.idbundle` attachment, or more than one |
| `object_too_big_for_carrier` | one object alone exceeds the profile        |

## 7. Alternatives weighed

- **QR byte mode with raw bytes:** about 3% denser, but scanner apps decode
  it as text with a guessed character set. Rejected for reliability.
- **Compression before base45:** would raise the QR ceiling for JSON-like
  objects. Rejected for now because the ratio depends on D3's byte encoding,
  which is not fixed, so any ceiling that relied on it would be aspirational.
  Worth revisiting once D3 fixes the encoding. It would also need a
  decompression-size limit.
- **Several codes chained into one bundle (QR structured append, or
  numbered codes):** many phone scanners do not support structured append,
  and a chain makes one lost code break the whole set. Splitting into
  independent bundles (§3) gives the same reach with no chain.
- **A signed "part k of n" field:** would tell a receiver when a split set
  is complete. Rejected because it ties parts together, which slice 1 §5
  avoided. Dangling links in slice 3's import report show what is missing
  instead.
- **exFAT or NTFS ceilings for media:** larger files, but not every desktop
  writes them. FAT32's limit is the one that works everywhere, and splitting
  covers the rest.
- **Bundle in the email body as text:** mail clients wrap and reflow body
  text, so an attachment is the only form that arrives byte for byte.
