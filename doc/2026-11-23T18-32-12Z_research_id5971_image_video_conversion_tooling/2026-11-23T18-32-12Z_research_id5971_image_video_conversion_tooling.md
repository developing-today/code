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
  with probing per rung, and states explicitly that it is *distinct from the conversion
  research*: the ladder chooses a tier, it does not say how media becomes that tier's bytes.
  That gap is this document.
- **Detection already exists in code.** `pkgs/id/crates/id-worlds/src/world_terminal.rs`
  probes and records a `kitty_graphics` capability bit (and the sixel bit) per connection;
  tier 1 in the #786 design (`doc/2026-11-01_design_id786.md`) is selected on exactly those
  bits. No graphics *encoder* or media converter exists anywhere in the repo yet.
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

| Use | Path | Verdict |
| --- | --- | --- |
| Still image | PNG payload, transmit once, place by ID | Cheap and exact; the primary tier |
| Short loop (sprite, reaction GIF-class) | Upload-once animation frames, terminal plays locally | Good at bounded geometry (≤ a few hundred px, tens of frames) |
| Long/streaming video | Per-frame transmission | **Not viable as a stored format.** Even PNG-per-frame at modest geometry is hundreds of KB per frame; viable only as a live presenter-side stream at small geometry/low fps, where the symbols/ASCII tiers usually win on cost anyway |

### 2.3 Terminal support matrix

Support as of this writing (November 2026), stills unless noted. "Partial" means a subset —
typically no animation, no unicode placeholders, or placement limits — which is why §2.1's
query, not a name table, is the authority at runtime.

| Terminal | Kitty graphics | Sixel | iTerm2 inline | Notes |
| --- | --- | --- | --- | --- |
| kitty | Full (reference) | — | — | Protocol author; animation + placeholders complete |
| Ghostty | Yes | — | — | Broad implementation; animation is the least mature part |
| WezTerm | Yes (stills) | Experimental | Yes | Kitty animation frames not composited; also the widest protocol coverage overall |
| Konsole | Partial | Yes | — | Kitty support is a subset; sixel is the reliable path |
| Contour | Recent/partial | Yes | — | Sixel primary |
| foot | — | Yes | — | Wayland only |
| xterm | — | Yes (vt340 mode) | — | Needs sixel-capable build/config |
| mlterm | — | Yes | — | |
| Windows Terminal | — | Yes | — | Sixel since 1.22 |
| iTerm2 | — | — | Yes (reference) | macOS |
| VS Code terminal | — | Recent | Yes | |
| mintty / Tabby / Hyper | — | — | Yes | |
| Alacritty, GNOME Terminal, Ptyxis | — | — | — | Symbols/ASCII tiers are the whole story here |

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

| Tool | What it is | Tiers | License | Usable as |
| --- | --- | --- | --- | --- |
| Hand-rolled APC encoder + `image` crate (Rust) | Decode/resize in `image`, PNG or RGBA+zlib payload, chunk, place | Kitty | `image` is MIT OR Apache-2.0 | **The build recommendation (§6)** |
| kitty `icat` kitten | The reference image display tool | Kitty | GPL-3.0 | Behavioral oracle; external tool only |
| chafa | The reference all-format converter (stills, animated GIF/WebP/APNG) | Kitty, sixel, iTerm2, symbols, ASCII | Library LGPL-3.0+; CLI GPL-3.0+ | Oracle + fallback reference; link the library only with LGPL obligations accepted, never bundle the CLI |
| timg | Image/video viewer-converter (FFmpeg decode) | Kitty, sixel, iTerm2, halfblock | GPL-3.0 | Oracle for video-over-kitty behavior; external only |
| viu | Rust image viewer | Kitty, sixel, iTerm2, halfblock | MIT | Prior art in our language; embeddable shape |
| term-image | Python library | Kitty, sixel, iTerm2 (+ placeholders) | MIT | Reference for placeholder handling |
| notcurses | Terminal UI library with blitters + video via FFmpeg | Kitty, sixel, iTerm2, symbols | Apache-2.0 | Embeddable, but a whole UI stack to adopt for one blitter |
| libsixel | The standard sixel encoder library | Sixel | MIT | **The sixel tier's encoder (§6)** |
| mpv (`--vo=kitty` / `--vo=sixel` / `--vo=tct`) | Media player with terminal video outputs | Kitty, sixel, text | GPL-2.0+ (LGPL parts) | Existence proof for live video at small geometry; external player, not a component |
| yazi image adapter | File manager's probe+encode layer | Kitty, sixel, iTerm2 | MIT | Prior art for probe/encode integration in Rust |

Two consequences:

1. **Nothing needs buying or bundling for the kitty tier.** A hand-rolled encoder over the
   `image` crate covers stills, animation frame sets, and placeholders with only permissive
   dependencies, and it can be conformance-tested against `icat`/chafa output.
2. **GPL tools are oracles, not parts.** icat, timg, mpv, and the chafa CLI may be run by a
   user who has them, and their behavior defines correctness — but none may be linked or
   shipped. The chafa *library* is the one borderline case (LGPL-3.0+): usable from a
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

| Tool | Shape | Stills | Video | Color | License | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| chafa (symbols mode) | Symbols + ASCII | Yes | Animated formats; video files via media loaders | Truecolor | LGPL-3.0+ lib / GPL-3.0+ CLI | The quality reference for the symbols rung |
| ascii-image-converter | ASCII + braille | Yes | — | ANSI color | Apache-2.0 | Go; the permissive prior art for the stills sampler |
| jp2a | ASCII | Yes (JPEG) | — | Mono / ANSI | GPL-2.0 | The classic jp2a-style pipeline of the brief; external only |
| video-to-ascii | ASCII / block chars | — | Yes (FFmpeg frames) | ANSI 256 | MIT | Python; exports a self-playing shell script of frames |
| mpv `--vo=tct`, mplayer `-vo aa/caca` | Text / symbols | — | Yes, in-player | tct truecolor; aa mono | GPL-2.0+ / aalib LGPL-2.0+ | Legacy lineage; proves decode+sample in one process |
| `vt` | Symbols + protocol tiers | — | Yes | Truecolor | (Go; cited by #6011) | Terminal video player riding the same ladder |

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
  stream at 200×60 and 15 fps is ~1.5 MB/s — *larger than the video it came from* at small
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

| Conversion | Placement | Why |
| --- | --- | --- |
| Format normalization, size/pixel/frame caps, metadata & payload stripping (decode-and-re-encode sanitization) | **Upload** | Viewer-independent; also the security checkpoint — assets enter by ID only after this |
| Poster frame / static preview derivative | **Upload** | Viewer-independent; one per asset |
| Web/share derivatives (ordinary image files for non-terminal surfaces) | **Upload** | Viewer-independent |
| Kitty / sixel / iTerm2 encoding | **Render** (presenter) | Output is a function of probed tier + geometry; PNG passthrough also means the "conversion" is nearly free at render time |
| Short-loop animation frame set (kitty `a=f`) | **Render**, from the stored original | Frame geometry is viewer-dependent; the upload-time alternative (one frame set per geometry bucket) multiplies storage for a niche path — revisit only if a hot asset class proves it |
| Symbols / ASCII sampling (stills) | **Render** | Geometry- and ramp-dependent; cost is negligible next to decode |
| Video decode + sampling (ASCII/symbols video) | **Render** | §4.2's arithmetic: a pre-rendered ASCII stream is geometry-locked and larger than the source video; decode is the real cost and is tier-independent, so it sits next to the sampler |
| Long-video "kitty streaming" | **Not offered** | §2.2: not a viable stored or streamed tier; video viewers get the symbols/ASCII tiers, or an ordinary video surface outside the terminal ladder |

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
