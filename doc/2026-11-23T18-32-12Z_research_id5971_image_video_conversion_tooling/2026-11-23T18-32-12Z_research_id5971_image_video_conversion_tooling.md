# Image/video conversion tooling: kitty graphics protocol and video-to-ASCII

Status: research record for #5971 (worlds design program #5955, statements S73–S74 of the
2026-11-04 brief). The pipeline recommendations in §6 and the placement decision in §5 are
**proposed, not decided** — they are inputs to the presentation-assets design #5970, whose
record ratifies or amends them. The survey facts (§2–§4) stand on their own and also feed the
licensing program (license checker #5985): every tool below carries its license, and §4.3
states the bundling rule the survey implies.

Scope guard, from the issue: this is graphics/media conversion only. #787 (kitty **keyboard**
protocol, slices A–F, landed) is a different protocol that happens to share the terminal; the
only contact point is that both are negotiated per connection by the presenter.

## 1. Where this starts: what already exists

Done check against main at the time of writing (2026-11-23), so the survey builds on the
record instead of redoing it:

- **Selection is already designed; conversion was not.** #6011 (closed) fixed the client-side
  rendering ladder — kitty graphics → sixel → iTerm2 inline → Unicode symbols → plain ASCII —
  with probing per rung, and states explicitly that it is _distinct from the conversion
  research_: the ladder chooses a tier, it does not say how media becomes that tier's bytes.
  That gap is this document.
- **Detection already exists in code.** `pkgs/id/crates/id-worlds/src/world_terminal.rs`
  probes and records a `kitty_graphics` capability bit (and the sixel bit) per connection;
  tier 1 in the #786 design (`doc/2026-11-01_design_id786.md`) is selected on exactly those
  bits. No graphics _encoder_ or media converter exists anywhere in the repo yet.
- **The boundary is already drawn.** The #786 design rules that the presenter owns graphics
  encoding, a guest may never supply kitty/sixel escape bytes, and the encoder is exercised
  with a bounded, test-owned image fixture. Conversion therefore lives presenter-side (or in
  a presentation-layer service acting for the presenter), never in guest/world code.
- **Assets are referenced by ID.** #5970's model is user media uploaded once and referenced
  by content ID, then degraded per viewer. The content-addressed media plane design
  (`doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane/`) supplies the storage
  and derivative-caching substrate §5's decision assumes.
- **The "IDF tools" named in the brief could not be identified as a tool family.** Neither
  the repo record nor public sources surface a maintained "IDF" video-to-ASCII converter
  lineage; the nearest real referent is the ANSI/textmode-art scene's static formats (the
  iCE Draw `.IDF` file format and its ACiD/iCE-era tooling), which are authored static art,
  not video converters. §4 surveys the actual video-to-ASCII field; if Drew has a specific
  IDF tool in mind, naming it reopens exactly one row of that table, not the survey.

## 2. The kitty graphics protocol

### 2.1 Mechanics that matter to a converter

The protocol is an APC escape sequence (`ESC _G <control> ; <payload> ESC \`):

- **Payload encodings.** `f=100` PNG (the terminal decodes; smallest wire size for stills and
  the only sane photographic path), `f=32` raw RGBA, `f=24` raw RGB. Raw pixel payloads may
  be zlib-compressed (`o=z`). Everything is base64 on the wire, so raw RGBA costs ~4/3 ×
  4 bytes per pixel before compression — a 1920×1080 frame is ≈11 MB base64 uncompressed.
- **Chunking.** A transmission is split into chunks of at most 4096 base64 characters, with
  `m=1` marking "more chunks follow" on all but the last. Converters must chunk; terminals
  reassemble before decoding, so chunk boundaries are invisible above the encoder.
- **Transmission mediums.** `t=d` direct (payload in the sequence), `t=f` file path,
  `t=t` temporary file (terminal deletes after reading), `t=s` POSIX shared memory. File and
  shared-memory mediums are local-only optimizations — over SSH/iroh, the id transport case,
  only `t=d` exists. A converter for this program emits `t=d` and treats the others as a
  possible local fast path, never a requirement.
- **Identity and placement.** Transmitted images carry an ID (`i=`); placements (`p=`, with
  cell geometry, z-index, and sub-cell offsets) reference stored images, so re-displaying or
  moving an image does not retransmit it. **Unicode placeholders** let an image occupy cells
  in the text stream (referenced by ID + diacritic row/column encoding), which is the mode
  that survives scrollback and text reflow.
- **Query before commitment.** `a=q` asks the terminal to validate format/size support
  without displaying; the response distinguishes "protocol present" from "this payload
  accepted" — the presenter's probe (#786 detection) should use the query, not TERM names.
- **Animation.** Frames transmit with `a=f` against an image ID, each frame optionally a
  sub-rectangle edit of the previous frame (crop region + offset), with a per-frame gap in
  milliseconds; `a=a` starts playback and the **terminal animates locally** from frames it
  already holds. This is the protocol's one video-shaped feature, and its economics are
  upload-once: a short loop costs its frames once, then zero bandwidth. Frame count and
  pixel size are bounded by terminal memory, so it fits sprite loops and short clips at
  bounded geometry — not feature video.

### 2.2 Stills vs animation vs video, honestly

| Use                                     | Path                                                 | Verdict                                                                                                                                                                                                                               |
| --------------------------------------- | ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Still image                             | PNG payload, transmit once, place by ID              | Cheap and exact; the primary tier                                                                                                                                                                                                     |
| Short loop (sprite, reaction GIF-class) | Upload-once animation frames, terminal plays locally | Good at bounded geometry (≤ a few hundred px, tens of frames)                                                                                                                                                                         |
| Long/streaming video                    | Per-frame transmission                               | **Not viable as a stored format.** Even PNG-per-frame at modest geometry is hundreds of KB per frame; viable only as a live presenter-side stream at small geometry/low fps, where the symbols/ASCII tiers usually win on cost anyway |

### 2.3 Terminal support matrix

Support as of this writing (November 2026), stills unless noted. "Partial" means a subset —
typically no animation, no unicode placeholders, or placement limits — which is why §2.1's
query, not a name table, is the authority at runtime.

| Terminal                          | Kitty graphics   | Sixel            | iTerm2 inline   | Notes                                                                            |
| --------------------------------- | ---------------- | ---------------- | --------------- | -------------------------------------------------------------------------------- |
| kitty                             | Full (reference) | —                | —               | Protocol author; animation + placeholders complete                               |
| Ghostty                           | Yes              | —                | —               | Broad implementation; animation is the least mature part                         |
| WezTerm                           | Yes (stills)     | Experimental     | Yes             | Kitty animation frames not composited; also the widest protocol coverage overall |
| Konsole                           | Partial          | Yes              | —               | Kitty support is a subset; sixel is the reliable path                            |
| Contour                           | Recent/partial   | Yes              | —               | Sixel primary                                                                    |
| foot                              | —                | Yes              | —               | Wayland only                                                                     |
| xterm                             | —                | Yes (vt340 mode) | —               | Needs sixel-capable build/config                                                 |
| mlterm                            | —                | Yes              | —               |                                                                                  |
| Windows Terminal                  | —                | Yes              | —               | Sixel since 1.22                                                                 |
| iTerm2                            | —                | —                | Yes (reference) | macOS                                                                            |
| VS Code terminal                  | —                | Recent           | Yes             |                                                                                  |
| mintty / Tabby / Hyper            | —                | —                | Yes             |                                                                                  |
| Alacritty, GNOME Terminal, Ptyxis | —                | —                | —               | Symbols/ASCII tiers are the whole story here                                     |

Multiplexers cut across every row: tmux passes kitty graphics only with passthrough
enabled and degrades placements; #6011 already records tmux passthrough as a recurring
breakage the ladder must handle. The converter consequence: never assume the probed
capability of the outer terminal survives a multiplexer unchanged — probe the session you
actually have.

## 3. Encoder and converter tooling landscape

The headline finding: **the kitty encoder is not a library problem.** The wire format is an
APC wrapper, base64, and 4096-character chunking around either a PNG byte stream or raw
pixels; every serious implementation hand-rolls it in a few hundred lines on top of a general
image crate for decode/resize/encode. What the field actually offers is (a) full converters
that bundle the whole ladder, useful as references and oracles, and (b) component libraries
for the non-kitty tiers. Licenses are the deciding constraint, per the licensing program.

| Tool                                           | What it is                                                          | Tiers                                 | License                         | Usable as                                                                                               |
| ---------------------------------------------- | ------------------------------------------------------------------- | ------------------------------------- | ------------------------------- | ------------------------------------------------------------------------------------------------------- |
| Hand-rolled APC encoder + `image` crate (Rust) | Decode/resize in `image`, PNG or RGBA+zlib payload, chunk, place    | Kitty                                 | `image` is MIT OR Apache-2.0    | **The build recommendation (§6)**                                                                       |
| kitty `icat` kitten                            | The reference image display tool                                    | Kitty                                 | GPL-3.0                         | Behavioral oracle; external tool only                                                                   |
| chafa                                          | The reference all-format converter (stills, animated GIF/WebP/APNG) | Kitty, sixel, iTerm2, symbols, ASCII  | Library LGPL-3.0+; CLI GPL-3.0+ | Oracle + fallback reference; link the library only with LGPL obligations accepted, never bundle the CLI |
| timg                                           | Image/video viewer-converter (FFmpeg decode)                        | Kitty, sixel, iTerm2, halfblock       | GPL-3.0                         | Oracle for video-over-kitty behavior; external only                                                     |
| viu                                            | Rust image viewer                                                   | Kitty, sixel, iTerm2, halfblock       | MIT                             | Prior art in our language; embeddable shape                                                             |
| term-image                                     | Python library                                                      | Kitty, sixel, iTerm2 (+ placeholders) | MIT                             | Reference for placeholder handling                                                                      |
| notcurses                                      | Terminal UI library with blitters + video via FFmpeg                | Kitty, sixel, iTerm2, symbols         | Apache-2.0                      | Embeddable, but a whole UI stack to adopt for one blitter                                               |
| libsixel                                       | The standard sixel encoder library                                  | Sixel                                 | MIT                             | **The sixel tier's encoder (§6)**                                                                       |
| mpv (`--vo=kitty` / `--vo=sixel` / `--vo=tct`) | Media player with terminal video outputs                            | Kitty, sixel, text                    | GPL-2.0+ (LGPL parts)           | Existence proof for live video at small geometry; external player, not a component                      |
| yazi image adapter                             | File manager's probe+encode layer                                   | Kitty, sixel, iTerm2                  | MIT                             | Prior art for probe/encode integration in Rust                                                          |

Two consequences:

1. **Nothing needs buying or bundling for the kitty tier.** A hand-rolled encoder over the
   `image` crate covers stills, animation frame sets, and placeholders with only permissive
   dependencies, and it can be conformance-tested against `icat`/chafa output.
2. **GPL tools are oracles, not parts.** icat, timg, mpv, and the chafa CLI may be run by a
   user who has them, and their behavior defines correctness — but none may be linked or
   shipped. The chafa _library_ is the one borderline case (LGPL-3.0+): usable from a
   separate process or with LGPL compliance for dynamic linking; §6 does not need it.

## 4. Video-to-ASCII and stills-to-ASCII

### 4.1 The field

"ASCII conversion" is three different output shapes with very different fidelity, and the
survey (and #6011's ladder, which has both a symbols rung and an ASCII rung) keeps them
separate:

- **Plain ASCII**: one luminance-ramp character per cell, optional ANSI color. Lowest
  fidelity, universal, tiny bandwidth.
- **Symbols**: halfblock/quadrant/sextant/braille glyphs with ANSI 256 or truecolor —
  halfblock doubles vertical resolution and carries foreground/background color per cell.
  This is what "good ASCII video" actually is in modern tools.
- **Braille**: 2×4 dot matrix per cell; best line-art/edge rendition, effectively monochrome
  per cell.

| Tool                                  | Shape                    | Stills     | Video                                           | Color                  | License                      | Notes                                                       |
| ------------------------------------- | ------------------------ | ---------- | ----------------------------------------------- | ---------------------- | ---------------------------- | ----------------------------------------------------------- |
| chafa (symbols mode)                  | Symbols + ASCII          | Yes        | Animated formats; video files via media loaders | Truecolor              | LGPL-3.0+ lib / GPL-3.0+ CLI | The quality reference for the symbols rung                  |
| ascii-image-converter                 | ASCII + braille          | Yes        | —                                               | ANSI color             | Apache-2.0                   | Go; the permissive prior art for the stills sampler         |
| jp2a                                  | ASCII                    | Yes (JPEG) | —                                               | Mono / ANSI            | GPL-2.0                      | The classic jp2a-style pipeline of the brief; external only |
| video-to-ascii                        | ASCII / block chars      | —          | Yes (FFmpeg frames)                             | ANSI 256               | MIT                          | Python; exports a self-playing shell script of frames       |
| mpv `--vo=tct`, mplayer `-vo aa/caca` | Text / symbols           | —          | Yes, in-player                                  | tct truecolor; aa mono | GPL-2.0+ / aalib LGPL-2.0+   | Legacy lineage; proves decode+sample in one process         |
| `vt`                                  | Symbols + protocol tiers | —          | Yes                                             | Truecolor              | (Go; cited by #6011)         | Terminal video player riding the same ladder                |

### 4.2 Quality, speed, color — the honest summary

- **Conversion is never the bottleneck.** Sampling a decoded frame to any of the three
  shapes is a resize plus a per-cell map — real-time at any terminal geometry on any modern
  CPU. **Decode is the cost** (FFmpeg-class), and it is identical for every tier, which is
  why §5 puts decode at render time next to the sampler instead of transcoding centrally.
- **Color fidelity** ranks: symbols+truecolor ≫ ANSI-256 ASCII > luminance-only ASCII ≈
  braille for photographic content (braille wins for line art). A pipeline that offers only
  the luminance ramp will look broken next to chafa's symbols output; the symbols shape is
  the tier to invest in, plain ASCII is its degradation, not a separate product.
- **Frame-diffing dominates bandwidth.** Emitting only changed cell runs per frame (the
  presenter's diff machinery from #786 already does this for cell grids) cuts ASCII-video
  bandwidth by an order of magnitude versus full-frame text; a pre-rendered full-frame ASCII
  stream at 200×60 and 15 fps is ~1.5 MB/s — _larger than the video it came from_ at small
  geometry. This kills the "convert video to ASCII at upload" idea on arithmetic alone (§5).

### 4.3 Licensing rule the survey implies

For the licensing program (#5985): converter components split cleanly into **linkable**
(MIT/Apache-2.0/LGPL-with-obligations: `image`, libsixel, viu-shape, ascii-image-converter
shape, notcurses) and **external-oracle-only** (GPL-2.0/3.0: icat, timg, jp2a, mpv, chafa
CLI). FFmpeg itself is LGPL-2.1+ in a default build and GPL when built with GPL components —
any future decode component must pin which build it means. §6's pipelines use only the
linkable set, so the conversion layer adds no new license-checker cases.

## 5. Conversion placement: the decision

The issue offers three placements: upload-time, client-side at render time, or both for
different tiers. The survey forces a sharper rule than "both": **placement follows what the
conversion's output depends on.**

- A conversion whose output is **viewer-independent** (same bytes for every viewer) belongs
  at upload time: paid once, cached by content hash, shared by all viewers and worlds.
- A conversion whose output is **viewer-dependent** (a function of the probed tier, the cell
  geometry, and the color depth of one specific session) cannot be done at upload time at
  all — the inputs don't exist yet. It belongs at render time, in the presenter that already
  owns probing (#786) and encoding (the #786 boundary rule).

Applied:

| Conversion                                                                                                    | Placement                            | Why                                                                                                                                                                                   |
| ------------------------------------------------------------------------------------------------------------- | ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Format normalization, size/pixel/frame caps, metadata & payload stripping (decode-and-re-encode sanitization) | **Upload**                           | Viewer-independent; also the security checkpoint — assets enter by ID only after this                                                                                                 |
| Poster frame / static preview derivative                                                                      | **Upload**                           | Viewer-independent; one per asset                                                                                                                                                     |
| Web/share derivatives (ordinary image files for non-terminal surfaces)                                        | **Upload**                           | Viewer-independent                                                                                                                                                                    |
| Kitty / sixel / iTerm2 encoding                                                                               | **Render** (presenter)               | Output is a function of probed tier + geometry; PNG passthrough also means the "conversion" is nearly free at render time                                                             |
| Short-loop animation frame set (kitty `a=f`)                                                                  | **Render**, from the stored original | Frame geometry is viewer-dependent; the upload-time alternative (one frame set per geometry bucket) multiplies storage for a niche path — revisit only if a hot asset class proves it |
| Symbols / ASCII sampling (stills)                                                                             | **Render**                           | Geometry- and ramp-dependent; cost is negligible next to decode                                                                                                                       |
| Video decode + sampling (ASCII/symbols video)                                                                 | **Render**                           | §4.2's arithmetic: a pre-rendered ASCII stream is geometry-locked and larger than the source video; decode is the real cost and is tier-independent, so it sits next to the sampler   |
| Long-video "kitty streaming"                                                                                  | **Not offered**                      | §2.2: not a viable stored or streamed tier; video viewers get the symbols/ASCII tiers, or an ordinary video surface outside the terminal ladder                                       |

**Caching makes the split cheap.** Derivatives — uploaded or rendered — are keyed
`(content hash, tier, geometry bucket, converter version)` in the content-addressed media
plane, so a render-time conversion that recurs (same asset, same terminal class, same size
bucket) is computed once per key, whoever computes it. Placement is then a deployment
choice per derivative class — a thin client may ask a presentation-layer service to render
its tier bytes — while the cache key, not the placement, is the invariant. This is also the
sense in which #5970's "basic conversion tools at the presentation layer" is satisfied: the
tools live at the presentation layer logically (presenter or its service), even when a
specific derivative was precomputed at upload.

**What upload-time conversion must never do** is pick a terminal tier. An upload pipeline
that bakes "the kitty version" of an asset has guessed a viewer, and every other viewer of
the same asset ID pays for the guess in quality or bandwidth.

## 6. Recommended converter pipeline per tier

One pipeline, three emission stages, all from the linkable license set (§4.3). The shared
front half — decode, normalize, resize to the negotiated geometry — is written once; tiers
differ only in the final encoder.

1. **Front half (all tiers).** Decode with the `image` crate (stills, animated GIF/WebP/APNG
   frames); enforce the upload-time caps again at read if the asset predates them; resize to
   the probed cell geometry × the terminal's cell pixel size; hand the encoder RGBA pixels
   plus, for stills, the original PNG bytes when no resize was needed.
2. **Kitty graphics tier.** Hand-rolled APC encoder: PNG payload (`f=100`) untouched when
   geometry already fits (zero re-encode); otherwise re-encode the resized frame as PNG, or
   RGBA+zlib (`f=32, o=z`) when the frame is one of a changing sequence. Chunk at 4096,
   transmit under an image ID, place by ID; unicode placeholders when the image must live in
   text flow. Short loops: transmit the frame set once (`a=f`, sub-rectangle edits where
   frames share regions), start playback (`a=a`), then stop sending bytes.
3. **ASCII video tier.** Decode frames (FFmpeg as an external process where its license
   posture is accepted per §4.3, or a platform decoder), sample each frame to the symbols
   shape (halfblock + truecolor by default; luminance-ramp ASCII as the degradation when
   color or glyph support is absent; braille for line-art mode), and emit through the
   presenter's existing cell-diff machinery — never as a pre-rendered text file.
4. **Static ASCII tier.** The same sampler, one frame. Stills only; an animated source uses
   its poster frame (upload-time derivative) unless the viewer negotiates the video tier.
5. **Fallback completeness (not separate products).** Sixel: libsixel encode of the same
   resized RGBA at render time. iTerm2: base64 passthrough of the stored file — the one tier
   with no real conversion, which is why it is cheap to keep on the ladder.

Conformance strategy: golden-fixture tests against `icat` and chafa outputs for the kitty
and symbols paths (the #786 design already prescribes a bounded, test-owned image fixture
for the encoder), plus the §2.1 query round-trip against at least kitty and WezTerm before
either is claimed as supported rather than merely detected.

## 7. Open questions and the follow-up

- **IDF (for Drew, on #5971):** §1 records that no tool family by that name could be
  identified. If a specific tool was meant, naming it adds one survey row; no recommendation
  above depends on it.
- **Cell pixel size discovery** is the encoder's weakest input: not every terminal reports
  `CSI 16t` / `XTWINOPS` truthfully, and a wrong cell size distorts every raster tier. The
  build slice should treat geometry as probed-with-fallback, like capability itself.
- **Terminal memory limits for animation frame sets** are undocumented per terminal; the
  frame-set path needs conservative default caps (pixels × frames) until measured.
- **First build slice** (filed separately, per the research-issue rule): the presenter-side
  kitty stills pipeline — front half + kitty encoder + query-based gating + golden fixtures —
  because it is the ladder's top rung, the cheapest tier to build, and the one every other
  tier's encoder reuses the front half of. ASCII/symbols sampling is the second slice; sixel
  via libsixel is third; animation frame sets last, behind the memory-cap measurement.

## Sources

- Kitty graphics protocol specification — https://sw.kovidgoyal.net/kitty/graphics-protocol/
- Kitty keyboard protocol (out of scope, for the boundary) — https://sw.kovidgoyal.net/kitty/keyboard-protocol/
- Chafa — https://hpjansson.org/chafa/ and https://github.com/hpjansson/chafa
- timg — https://github.com/hzeller/timg
- viu — https://github.com/atanunq/viu
- term-image — https://github.com/AnonymouX47/term-image
- libsixel — https://github.com/saitoha/libsixel
- notcurses — https://github.com/dankamongmen/notcurses
- ascii-image-converter — https://github.com/TheZoraiz/ascii-image-converter
- jp2a — https://github.com/cslarsen/jp2a
- video-to-ascii — https://github.com/joelibaceta/video-to-ascii
- mpv terminal video outputs — https://mpv.io/manual/master/ (vo_kitty, vo_sixel, vo_tct)
- Terminal protocol support matrices consulted for §2.3 — https://aeshell.github.io/docs/readline/terminal-images/ and https://www.termium.dev/docs/terminals/
- In-repo: #5955 (program), #5970 (presentation assets), #6011 (rendering ladder),
  #786 + `doc/2026-11-01_design_id786.md` (capability tiers and presenter boundary),
  #5985 (license combination checker), `doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane/` (derivative substrate),
  `pkgs/id/crates/id-worlds/src/world_terminal.rs` (existing capability probes).

---

## Amendment — 2026-11-23 (later the same day): the kitty encoder survey

_Added after Drewry challenged §3's headline conclusion, naming kittage, little-kitty,
rasteroid, zellij, ghostty, and kitty-graphics-protocol-rs as implementations the survey
should have covered. He was right. This section corrects the record; the original text
above is unchanged._

### A.1 What was wrong

§3's headline — "the kitty encoder is not a library problem" — was wrong as stated, and
§6.2's hand-roll-the-encoder recommendation rested on it. The original survey compared
**applications and converters** (icat, chafa, timg, viu, term-image, notcurses, libsixel,
mpv, yazi) and never looked for **protocol libraries**: crates whose whole job is composing
kitty graphics commands. Those exist, in Rust, today, and two of them cover most or all of
what §6.2 proposed to hand-roll. What survives the correction: §2's protocol mechanics,
§4's placement analysis, §5's conversion-tool survey, §6.1/§6.3, §7's sequencing, and
§6.2's wire-format description of what an encoder must emit. What changes is only the
_build_ decision — revised in A.3.

Two factual updates to §2.3 fell out of the same pass:

- **Ghostty 1.4** (2026-10-27) completed the published kitty graphics specification,
  animation included (Ghostty's own protocol-status page: "100% of published protocol:
  Yes (completed in version 1.4)"). The matrix row's "animation is the least mature part"
  was already stale when written.
- **Zellij 0.45** (2026-08) implements kitty graphics as a multiplexer: it parses pane
  clients' APC streams, tracks placements per pane (surviving resize, scroll, and floating
  panes), and re-emits to the host terminal, recompressing with zlib (`o=z`) when the host
  supports it. That is a concrete, shipped instance of the multiplexer behavior §2.3's
  caveat describes, and the strongest reason the presenter's probe — not the TERM name —
  stays the authority.

### A.2 The survey

Checked 2026-11-23 against crates.io, docs.rs, lib.rs, and project documentation.
"Coverage" is measured against what #13623 needs first (direct `t=d` transmission,
4096-character chunking, image IDs, placements, an `a=q` query/response round-trip) and
what §7 needs later (unicode placeholders, animation).

| Project                                   | Kind                                         | Language   | License                                                             | Coverage                                                                                                                                                                                                                                                                                                                                                                    | State                                                                                            | Fit for #13623                                                                                                                                                                                                                                           |
| ----------------------------------------- | -------------------------------------------- | ---------- | ------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| kittage 0.4.0                             | client encoder library                       | Rust       | **MPL-2.0**                                                         | Everything except animation frames: all mediums (direct/file/temp/shared-memory), configurable chunk size, zlib compression, image IDs/numbers, placements incl. relative placements, virtual placements (unicode placeholders), delete, query + response reading (sync and async), tmux passthrough                                                                        | 15 releases since 2025-08, latest 2026-04; ~13K downloads                                        | Fullest fit of anything surveyed — held only by license class (A.3)                                                                                                                                                                                      |
| little_kitty 0.0.3 (crate `little-kitty`) | client encoder library, low-level            | Rust       | **MIT OR Apache-2.0**                                               | Generic control-key composition; its published docs demonstrate `a=T` transmit-and-place with `f=100`, file payloads, image IDs, `U=1` placeholder registration, `a=q` with expected response, and a support probe; tmux wrapping; response reader. No chunk-splitting helper appears in its documented surface (`write_base64` emits a payload whole) — verify at adoption | First published 2026-03; 3 releases; ~1K downloads                                               | **Recommended base (A.3)** — permissive under §4.3 as written; the layer it provides is ~1.1K lines, thin enough to vendor under its own terms if it stalls                                                                                              |
| kitty-graphics-protocol 0.1.3             | client encoder library                       | Rust       | MIT                                                                 | Claims everything: RGB/RGBA/PNG, chunked transfer (default 4 KB), animation, placeholders, placements, detection                                                                                                                                                                                                                                                            | All 4 versions published on a single day (2026-02-28); docs.rs fails to build it; ~860 downloads | Not adoptable on current evidence — claims unverified, no maintenance signal; its `serialize_chunked` design is a useful reference                                                                                                                       |
| rasteroid 0.4.0                           | encoder library (under the mcat viewer)      | Rust       | MIT                                                                 | Kitty transmit-at-cursor only (`encode_image`, `encode_frames` for redraw-style animation); no image-ID, placement, placeholder, or query API                                                                                                                                                                                                                               | Active: 14 releases, mcat maintained                                                             | Does not cover #13623's transmit-by-ID + placement scope                                                                                                                                                                                                 |
| ratatui-image 11.x                        | encoder library (ratatui widget)             | Rust       | MIT                                                                 | Kitty backend transmits RGBA in chunks and renders **only** via unicode placeholders written into ratatui's cell buffer                                                                                                                                                                                                                                                     | Very active; hundreds of dependent crates; cross-terminal screenshot QA                          | Architectural mismatch: pkgs/id does not use ratatui, and #13623 needs direct placement by ID. Field reports note its placeholder path does not render in WezTerm — at least one downstream project hand-rolled direct placement for exactly that reason |
| viuer 0.11                                | encoder library (under the viu viewer)       | Rust       | MIT (lib.rs also lists LGPL-3.0-or-later, via its sixel dependency) | Viewer-grade kitty: print at cursor; no IDs, placements, or placeholders                                                                                                                                                                                                                                                                                                    | Maintained (2025-12)                                                                             | Same class as the original survey's viu row; not #13623 scope                                                                                                                                                                                            |
| yazi-adapter 26.x                         | application adapter (Yazi file manager)      | Rust       | MIT                                                                 | Production kitty encoding incl. chunking and placeholders, inside Yazi's preview pipeline                                                                                                                                                                                                                                                                                   | Released with Yazi (latest 2026-05)                                                              | Prior art / behavioral reference, as the original survey had it; its API is Yazi's adapter surface, not a general encoder                                                                                                                                |
| charmbracelet/x/ansi `kitty`              | client encoder + decoder library             | Go         | MIT                                                                 | Full: chunking (`MaxChunkSize`), all actions incl. animation frames, placeholders, complete delete taxonomy, plus a payload decoder                                                                                                                                                                                                                                         | Actively maintained (v0.10.x)                                                                    | Wrong language for pkgs/id; the best cross-language cross-check for #13623's golden fixtures                                                                                                                                                             |
| zellij 0.45+                              | terminal multiplexer (protocol intermediary) | Rust       | MIT                                                                 | Parses client kitty streams, tracks placements per pane, re-emits to host; `o=z` recompression when the host supports it                                                                                                                                                                                                                                                    | Current (0.46, 2026-11)                                                                          | Not a library and not published as one; evidence for §2.3's multiplexer caveat and a placement-tracking reference                                                                                                                                        |
| ghostty 1.4                               | terminal (renderer)                          | Zig        | MIT                                                                 | Project claims 100% of the published spec incl. animation and relative placements                                                                                                                                                                                                                                                                                           | Released 2026-10-27                                                                              | Conformance target and reference implementation; terminal-side, not a client encoder                                                                                                                                                                     |
| bboxy450_kitty_graphics 0.2.0             | terminal-side protocol state machine         | Rust       | non-standard                                                        | Chunked assembly, RGB/RGBA/PNG, display/delete/query; explicitly no file/shared-memory mediums, animation, or placeholders                                                                                                                                                                                                                                                  | Published 2026-11-13; 40 downloads                                                               | Terminal-side; completeness only                                                                                                                                                                                                                         |
| xterm.js addon-image                      | terminal-side renderer                       | TypeScript | MIT                                                                 | Full kitty command parsing/rendering in the browser terminal                                                                                                                                                                                                                                                                                                                | Maintained with xterm.js                                                                         | Terminal-side; completeness only                                                                                                                                                                                                                         |

Nothing in the original §3 table was a protocol library — every entry was a converter or
an application — which is precisely the gap. Those entries stand as converter/application
prior art; this table is the missing layer.

### A.3 Revised recommendation (replaces §6.2's build decision)

**The recommendation flips: build #13623 on a library. Hand-roll only the pieces no
permissively-licensed library provides.**

1. **Adopt `little-kitty` 0.0.3 (MIT OR Apache-2.0)** as the presenter's kitty
   composition layer: control-key command building, the `a=q` query/response round-trip
   the capability gate needs, placeholder registration for §7's later stages, and tmux
   wrapping. It is the only permissively-licensed option whose fit for #13623's scope
   could be verified from its published documentation. It is young (v0.0.3, first
   published 2026-03) — accepted knowingly: the layer is small, the license permits
   vendoring, and the alternative candidates are either license-blocked (kittage) or
   unverifiable (kitty-graphics-protocol).
2. **Keep in presenter code** (unchanged from §6.2): PNG passthrough via the `image`
   crate, zlib compression for `f=32 o=z`, and — unless little_kitty's execution path is
   found to split payloads internally, which its published docs do not show — the
   4096-character chunk loop with `m=` continuation. #13623's first step is that
   verification, and its golden fixtures (against icat/chafa output) now double as the
   verification that the library emits §6.2's wire format byte-for-byte.
3. **Hold `kittage` 0.4.0 as the named alternative.** It is the fullest implementation
   surveyed — it would retire more hand-rolled surface than little_kitty (chunking,
   mediums, placeholders, relative placements, response streams) — but it is **MPL-2.0**:
   weak, file-level copyleft, outside §4.3's linkable list as written. If the licensing
   program (#5985) rules MPL-2.0 into the same with-obligations class §4.3 already grants
   LGPL, kittage becomes the better buy and #13623 should switch before the encoder
   hardens. This amendment does not make that ruling.
4. **What does not change:** §6.2's wire spec (it now describes what the library must
   emit, and the fixtures still verify exactly that); §7's sequencing; the probe-gated
   posture (`a=q` before commitment, never TERM names); and §3's _converter_ conclusions
   (chafa/libsixel/notcurses remain the conversion-side story — this correction concerns
   only the protocol-encoder layer).

### A.4 Sources

crates.io API records and docs.rs/lib.rs pages for kittage, little-kitty,
kitty-graphics-protocol, rasteroid, viuer, ratatui-image, yazi-adapter, and
bboxy450_kitty_graphics (all read 2026-11-23); kittage `DisplayConfig` documentation
(virtual placement, relative placements) and `Medium`/`Compression` types; little_kitty
crate documentation (worked examples for transmit-and-place, placeholder registration,
query with expected response, support probe; `KittyCommandWriter` surface); Ghostty
protocol-status documentation and 1.4 release notes; Zellij 0.45.0 release notes and
configuration documentation (`support_kitty_graphics_protocol`); charmbracelet/x/ansi
`kitty` package documentation (pkg.go.dev).
