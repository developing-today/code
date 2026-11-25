# Inventory: semantic cell tags, slice 1 (buffer, tiers, fallback chain, per-world needs)

Date: 2026-11-25 21:30 UTC
Status: inventory record; decides nothing
Design record: `doc/2026-11-24T05-55-23Z_design_semantic-cell-tags/2026-11-24T05-55-23Z_design_semantic-cell-tags.md` (called "the record" below), §5 row 1
Prior #6010 record: `doc/2026-11-17_design_id6010_semantic_cell_tag_model.md` (called "the prior record"); #5970 cites it as the #6010 spec
Tracking: [developing-today/code#15244](https://github.com/developing-today/code/issues/15244)
Design issue: [#6010](https://github.com/developing-today/code/issues/6010); related [#5970](https://github.com/developing-today/code/issues/5970), [#786](https://github.com/developing-today/code/issues/786), [#14731](https://github.com/developing-today/code/issues/14731)
Baseline: detached checkout of `main` at the time of writing; read-only, nothing compiled or run.

Every fact below cites `file:line` and states only what the code shows. Section 5 holds candidates, each marked as a proposal. Vocabulary, flag contents, `sys.*` contents and field widths are Drew's decisions (record §8 items 1 to 3).

Path keys used in the tables:

| Key | Path |
| --- | --- |
| WP | `pkgs/id/crates/id-core/src/world_presentation.rs` |
| SC | `pkgs/id/crates/id-core/src/screen_codec.rs` |
| PP / PA / PF | `pkgs/id/crates/id-core/src/presentation_{policy,asset,filter}.rs` |
| WT | `pkgs/id/crates/id-worlds/src/world_terminal.rs` |
| WS / WC | `pkgs/id/web/src/world-surface.ts` / `pkgs/id/web/src/screen-codec.ts` |
| NR | `pkgs/id/crates/id-native-shell/src/render.rs` |
| RS | `pkgs/id/examples/roc-world/Screen.roc` (the Roc guest SDK screen helper) |

## 1. Screen cell buffer as shipped

### 1.1 The cell

`ScreenCell` (WP:1627-1644) is `#[serde(deny_unknown_fields)]` (WP:1629) with exactly six fields:

| Field | Type / width | Meaning in code |
| --- | --- | --- |
| `symbol` | `String`, one grapheme | "One safe Unicode grapheme, or empty for a wide glyph's continuation cell" (WP:1631-1632) |
| `fg` | `u8` | xterm-256 index, zero is presenter default (WP:1633-1634) |
| `bg` | `u8` | xterm-256 index, zero is presenter default (WP:1635-1636) |
| `bold` | `bool` | (WP:1637-1638) |
| `columns` | `u8`, values 0, 1, 2 | zero marks a wide-glyph continuation (WP:1626, WP:1639-1640) |
| `token` | `Option<StyleToken>`, omitted when `None` | "Semantic intent, if supplied by the guest" (WP:1641-1643) |

Validation (`is_valid`, WP:2323 onward): a continuation cell must follow a two-column head with equal `fg`, `bg`, `bold` and `token` and an empty symbol (WP:2336-2354); a head must be one grapheme, control-free, with Unicode width equal to `columns` (WP:2355-2362).

### 1.2 Wire shapes

There are two shapes, and tags would need both:

- **Guest document** (`ScreenPayload`, WP:1656-1668, `deny_unknown_fields`): `kind: "screen"`, `version`, `width`, `height`, `rows: [String]`, optional `styles: [[StyleSpan]]`, optional `tokens: [[TokenSpan]]`. `StyleSpan` is `{x, length, fg, bg, bold}` (WP:1670-1678); `TokenSpan` is `{x, length, token}` (WP:1680-1686). Both are `deny_unknown_fields`. The parser expands rows into cells (WP:1899-1947), applies style spans (WP:1949-1979), then token spans, which also overwrite `fg` and `bold` from the token's projection (WP:1981-2008, projection at WP:2001).
- **Host-normalized surface**: `WorldSurface` serialized with `kind` tag (WP:113-142), which for a screen is `ScreenSurface {version, width, height, cells}` (WP:754-767, `deny_unknown_fields`). The native shell notes it receives this `cells` form, "NOT the guest's `rows` document" (`id-native-shell/src/session.rs:65-73`).

Limits: `MAX_SCREEN_WIDTH` 120, `MAX_SCREEN_HEIGHT` 50, `MAX_SCREEN_CELLS` 6000, `MAX_SCREEN_STYLE_SPANS` 4096 (WP:65-71). The span limit is shared: style spans plus token spans together must not exceed 4096 (WP:1881-1887). Binary frames are capped at 1 MiB (`MAX_SCREEN_FRAME_BYTES`, SC:29).

Versioning: guest screens must be `version == 1` (WP:1849-1853); `from_cells` always builds version 1 (WP:791); the web renderer refuses anything but version 1 (WS:696). Patches and diffs require equal versions (WP:1535, WP:1575).

### 1.3 Extension points: none today

The record says tags "ride in the buffer's extension fields" (record §4.4). **No extension field exists.** Every screen structure is `deny_unknown_fields`: `ScreenSurface` (WP:756), `ScreenPatch` (WP:1507), `CellRun` (WP:1521), `ScreenCell` (WP:1629), `ScreenPayload` (WP:1657), `StyleSpan` (WP:1671), `TokenSpan` (WP:1681), and the binary envelopes (SC:32, SC:40, SC:109, SC:124, SC:132). There is no map, no reserved bits, and no free slot. The only optional field is `token`, a closed seven-value text-semantic enum (WP:1599-1611) that the prior record keeps separate from tags (prior record §1 table). The web decoders also check keys exactly: `decodeCell` rejects any key outside the six (WC:64-79).

Consequence: carrying tags needs either a new screen version (2) with new optional cell fields and a guest-side span channel, or a new optional field accepted by every validator in lockstep (Rust `parse_surface` and `is_valid`, WC `decodeCell`, WS `renderWorldSurface`, native `from_cells` revalidation at `session.rs:80-84`). Either is a format change. The record's §8 default ("nothing wider than the buffer's extension capacity without Drew's sign-off") therefore means any v1 tag field needs Drew's sign-off, because the capacity today is zero.

### 1.4 Patches

`ScreenPatch {version, width, height, runs}` (WP:1507-1518) carries `CellRun {start, cells: Vec<ScreenCell>}` (WP:1521-1528). `diff` compares whole cells with `==` (WP:1540-1541) and `apply` replaces whole cells (WP:1574-1590). The web applies runs the same way (WS:29-51). So if tags live inside `ScreenCell`, a tag-only change produces a patch with no patch-format change; if tags ride in a side table, patches need their own channel.

### 1.5 StyleToken and its adapter table

Tokens: `emphasis, muted, heading, code, warning, success, link` (WP:1602-1611). The adapter table is documented in the module header (WP:41-54) with terminal, 1-bit and text columns, and coded only for the terminal column (`ansi_style`, WP:1615-1625). The 1-bit column ("on" or "dither") has no implementation: a repo-wide search finds "dither" only in that header. Consumers: the web renderer reads only `fg`, `bg`, `bold` (WS:811-814); the native renderer reads `fg`, `bg`, `bold` into its model (NR:74-90) and draws only `fg` (NR:351); `LogicalCell` for primary-mode painting drops `token` (WP:2044-2050). No shipped world or guest SDK emits `tokens` (section 4).

### 1.6 Other surface kinds

All share the rule that `WorldSurface::text_projection` is exhaustive with no wildcard (WP:144-159).

| Kind | Shape (closed sets) | Text projection |
| --- | --- | --- |
| `text` | `TextSurface {version, width, lines}` (WP:625-638) | lines wrapped to width (WP:691) |
| `form` | fields and actions, closed field kinds (WP:164-250) | numbered field lines (WP:417) |
| `scene` | nodes with `SceneShape` box/sphere/plane/point (WP:846-893) | one line per node (WP:995) |
| `audio` | cues with `AudioFormat` wav/ogg/mp3 (WP:1023-1066) | one line per cue (WP:1127) |
| `widget` | controls button/toggle/link (WP:1153-1216) | one line per control with kind (WP:1304) |
| `media` | items with `MediaSort` image/audio/video (WP:1338-1392) | one line per item (WP:1477) |
| `screen` | cell grid above | row symbols, continuation cells skipped, trailing blanks trimmed (WP:2150-2171) |

No shipped world emits form, scene, audio, widget or media (a search of `apps/` and `examples/` finds none). Only `screen` is a cell grid, so this inventory scopes tags to `screen`.

## 2. Tiers (#786) as coded

| Tier | Selection evidence (WT:60-68) | Screen rendering |
| --- | --- | --- |
| Tier1 | (kitty graphics or sixel) and synchronized output | Tier 2 grid inside sync brackets (WT:701-707); presenter-owned images only, never guest payloads (WT:729-750) |
| Tier2 | cursor position report and 256-colour | full ANSI frame then row diffs (WT:684-697) |
| Tier3 | anything else, or a conflicted probe (WT:185-193) | `text_projection` then line deltas (WT:644-663) |

`DisplayMode` is auto, alternate, primary or text, negotiated per session via `ID_DISPLAY` (WT:82-145). Alternate and primary force at least Tier 2; text forces Tier 3 (WT:157-171). Primary mode paints with `paint_primary` (WP:2272) over `LogicalCell`, which has no token.

Other clients: the web shell draws a CSS grid of spans with xterm colours and bold (WS:694-820), so it is a Tier 2-equivalent cell renderer with no tier enum; the native Bevy shell builds a `ScreenGrid` and draws foreground only, bold "not yet visually distinct" (NR:1-20, NR:351); the TUI shell targets the text surface (`id-tui-shell/src/lib.rs:5-9`); the hardware shell renders forms only (`id-hw-shell/src/lib.rs:1-28`).

**No 1-bit or dither mode is implemented** in any presenter. PF keeps its own `Tier` enum (Tier1/2/3) mirroring the terminal tier, with filter chain bounds 8/4/0 (PF:34-55).

## 3. #5970 fallback chain as coded

- **Rungs**: `Rung::{Rich, Static, Tileset, Glyph}`, richest first; "Glyph. Always available" (PP:51-63).
- **Policy**: `PresentationPolicy {assets, filters, shaders, fallback}` (PP:337-348); `admit` caps by `max_rung`, drops to `Tileset` on denied uploads and to `Static` on denied filters (PP:459-489).
- **Fallback overrides**: `FallbackOverrides {world_glyphs: BTreeMap<String, Glyph>, world_substitutes, tileset}` (PP:318-333). `world_glyphs` is documented as "Themed glyphs by cell kind" (PP:324) but **the key is a free `String`**; validation checks only the entry count (PP:425-427). The module example key is `"tree"` (PP:14), which is in neither record's kind set. The keys await the kind vocabulary this program defines.
- **Glyph override validation**: non-empty, at most 16 bytes, no control characters (PP:45, PP:234-244). It does not require one grapheme or width 1 to 2, which `ScreenCell` does (WP:2355-2362), so a valid override such as `"/A\\"` cannot occupy one cell.
- **Assets and filters**: immutable `asset@revision` with declared parts (PA:1-30, PA:234-292); named filter registry with capability and rung fall (PF:1-30).
- **Not yet built**: the fallback resolver is "a later slice" (PP:33-35, PP:319-320, PA:23-26). `PresentationPolicy` has no consumer outside id-core tests (only PF:546-560 uses it). No app declares a policy (`apps/*/app.toml` has no presentation section).
- **Design doc**: `doc/2026-11-25T01-25-06Z_design_id5970_user_presentation_assets.md` §4 keys tiles by `(kind, form, material)` and glyphs by "#6010's total kind to glyph table" (lines 64-68), citing the prior record.

## 4. Per-world presentation needs

Sizes are frame dimensions. Unless noted, colours are xterm indices in `fg`, `bg` is 0, and no world sets `token`. Grid is "semantic cell to screen cell" width.

| World | Surface | What cells represent (glyph, colour) | Grid | Tag candidates it would need |
| --- | --- | --- | --- | --- |
| `apps/checkers` | screen 28x13 (main.roc:733-737) | dark empty square `.`, light square blank; red man `o`/king `O` fg 203, black man `x`/king `X` fg 245, king bold (785-809); selected piece `[ ]` fg 46 bold, cursor `[ ]` fg 226 bold (764-765, 811-819); HUD, rank/file labels, status text (753-769) | 3 screen cells per square | kind square (dark/light), kind piece with side and form man/king; flags cursor, selected; text for chrome |
| `apps/chess` | screen 28x13 (1014-1018) | white `KQRBNP` fg 231 bold, black `kqrbnp` fg 245 (1066-1106); same `.` squares and `[ ]` cursor/selected (1110-1116); check, promotion, mate only as status text (1118-1137) | 3 per square | as checkers plus forms king, queen, rook, bishop, knight, pawn; candidate `in_check` or highlighted (not drawn today) |
| `apps/space-invaders` | screen 40x22 (49, 64) | invaders 2-cell `<>` `/\` `[]` `{}` by row, fg 196/208/226/46 (430, 452-474); bombs `:` fg 201 (435); player shot `\|` fg 231 bold (437); player `/A\` fg 51 bold (441); ground `=` fg 240 (442); banners fg 46/196 (444-446); no shields | 2 cells per invader, 3 for player | creature (player, invader, row type), effect/projectile (shot, bomb), terrain (ground), text; hostile vs friendly |
| `apps/tic-tac-toe` | legacy text, one line (44-53) | board string `X`, `O`, `-` plus `plays=` and `next=`/`winner=` | none | none until it emits a screen; would need square, piece X/O |
| `examples/territory` | screen 24x19 via `render_rows_json` (61-72) | claimed cells `A`..`H`, fg per seat 10,12,11,13,14,9,15,8 bold (293-305); empty `.` fg 8 (321-329, 30); a seat's cursor replaces the cell with `a`..`h` on a seat bg (307-319, 248-252); box edges, scores | 1 per cell | kind terrain/territory with owner identity; flag cursor (today the cursor hides the owner beneath it) |
| `examples/roc-burrow` | screen 36x9 boxed (157-167) | digger `A`, rock `#`, exit `X`, soil `s`, cleared `.`, all default colour (172-184, 35, 38) | 1 per cell, space-separated | creature, terrain with blocking (rock), diggable soil, exit with open/locked |
| `examples/rust-colony` | screen 32x6 (385-410) | worker `@`, ore `*`, stockpile `S`, floor `.`, default colour (330-340); legend line spells the key (402) | 1 per cell, space-separated | creature, item (ore), structure (stockpile), terrain |
| `examples/rust-orbit` | screen 32x8 (464-498) | cannon `A`, target `T`, shot `\|`, empty `.`, default colour (366-376) | 1 per cell, space-separated | creature (cannon, target), effect (shot) |
| `examples/rust-cells` | screen 24x7 (114-122) | alive `#`, dead `.` (49-55) | 1 per cell | kind cell with state alive/dead |
| `examples/roc-grid` | screen 24x6 (25-32) | raw grid bytes as glyphs fg 2 bold; non-UTF-8 byte shows `?` (35-38) | 1 per cell | generic grid cell; shows a world-side `?` the prior record forbids at client level |
| `examples/roc-mind-map` | screen 40x10 boxed (115-123) | node `id name` labels at positions, outcome and link lines, default colour | label spans | text only; node and link kinds if ever tagged |
| Counters: `roc-counter`, `roc-screen`, `rust-counter`, `zig-counter` | screen 20x3 boxed, `count=N` (roc-counter 33-34; rust-counter 41-48, 70-73; zig-counter 76-78) | ASCII box `+-\|` (RS:110-127) and text | n/a | none beyond text/chrome |
| Text-only: `roc-text-counter`, `lounge`, `roc-chat`, `roc-cap-demo`, `rust-sketch` | legacy UTF-8 text (roc-text-counter 19-20; lounge 61-63; roc-chat 81-83; roc-cap-demo 36-44; rust-sketch 141-142) | status lines | none | none |
| Not worlds | `first-app-poc` (README only), `roc-import-policy` (fixtures), `roc-world`, `rust-world`, `zig-sdk` (SDKs), `session_checkpoint_poc.rs`, `roc-toolchain.sh` | | | |

Observations across the table:

- The three guest SDKs emit only `rows` and `styles`, never `tokens` (RS:136-151, `rust-world/src/screen.rs:151`, `zig-sdk/src/screen.zig:503`). Tags therefore need SDK support in Roc, Rust and Zig, not only host support.
- `view` and `layout` receive no viewer identity (`roc-world/platform.roc:6-7`, `rust-world/src/lib.rs:44-46`); checkers says "every mode and viewport sees the same frame" (checkers main.roc:745-747). Viewer-relative flags such as `owned_by_viewer` cannot be authored today.
- Selection, cursor and side are carried only by colour or by neighbouring bracket cells; no world marks last move, legal targets, damage or animation.

## 5. v1 vocabulary candidates (all proposals; decider Drew, record §8)

Each row gives the ASCII fallback sketch (glyph, xterm fg, bold) a client table could use when a world supplies no authored glyph, so the ASCII tier stays complete.

### 5.1 Kind (exactly one per cell). Record default: derive from shipped buffer plus #5970 examples; smallest set covering current worlds.

| Candidate | Used by (section 4) | In prior record? | ASCII sketch |
| --- | --- | --- | --- |
| `empty` | every world's blank cells | yes | ` ` default |
| `terrain` | squares, floor, ground, territory, soil, rock | yes | `.` fg 245; `#` if `blocking` |
| `structure` | stockpile, exit | yes | `+` fg 250 |
| `creature` | player, invaders, digger, worker, cannon, target | yes | `@` default bold |
| `item` | ore | yes | `*` fg 226 |
| `effect` | shots, bombs | yes | `\|` fg 231 |
| `marker` | cursor/selection brackets if kept as cells | yes | `[` `]` fg 226 bold |
| `piece` (open question) | checkers, chess, tic-tac-toe marks | no; prior set would file pieces under `creature` or `item` | `o` default bold |
| `text` (open question) | HUD, labels, box chrome, status lines in every screen world | no; prior record has no kind for chrome | the symbol itself, default |

`fluid` and `vegetation` (prior record §2) have no current user; the record's "smallest set" default would defer them. Record §4.1's suggestion (terrain, creature, item, effect, empty) lacks a home for board pieces, markers and chrome.

### 5.2 State flags. Record default: smallest set covering current worlds.

A candidate 8-bit mask, ordered by how many worlds show the state today:

| Bit | Flag | Shown today by | ASCII sketch |
| --- | --- | --- | --- |
| 0 | `cursor` | checkers, chess, territory | bg 7, bold (one-cell form of the bracket/inverse cue) |
| 1 | `selected` | checkers, chess | fg 46 bold |
| 2 | `blocking` | burrow rock | glyph `#` |
| 3 | `hostile` | space-invaders (by colour only) | fg 196 |
| 4 | `friendly` | space-invaders player | fg 51 |
| 5 | `open` | burrow exit opens after quota | locked `X` fg 245; open `X` fg 46 bold |
| 6 | `highlighted` | no current user (candidate for last move, check, legal target) | bold |
| 7 | `animated` | no current user (motion is re-rendered per tick) | none; must be ignorable at ASCII |

Prior record flags not observed in any world: `damaged`, `destroyed`, `locked` (covered by `open` clear), `lit`, `occluded`, `hidden`, `neutral`, `interactive`, `owned_by_viewer` (blocked by the no-viewer ABI above).

### 5.3 `sys.*` core tags. Record default: colour only.

Colours the worlds actually use (section 4) mapped to the prior record's colour words: 196/203/9 red, 208 orange, 226/11 yellow, 46/10/2 green, 12 blue, 231/15 white, 245/240/8 grey. **Cyan (51, 14) and magenta (201, 13) are used but absent** from the prior record's ten colours; a v1 `sys.color.*` set covering current worlds would add them or map them to blue and purple. Sketch: each `sys.color.X` sets `fg` to the client's index for X (prior record §2 makes this a client table), with glyph unchanged.

### 5.4 `world.*` examples

`world.man`, `world.king`, `world.queen`, `world.rook`, `world.bishop`, `world.knight`, `world.pawn`, `world.side_red`, `world.side_black`, `world.invader_row_0..3`, `world.soil`, `world.ore`, `world.stockpile`, `world.exit`, `world.alive`. ASCII sketch for each is the world's current glyph and colour from section 4 (for example `world.king` plus `world.side_red` reads `O` fg 203 bold). Under the record's extension rule each needs a full per-tier row before it ships.

Namespace spelling differs between records: the prior record reserves `world/<name>` (prior record §2) and the record uses `world.*` and `sys.*` (record §4.1). The spec slice must pick one.

## 6. Gaps and risks

1. **No extension field** (section 1.3). Every screen struct and both web decoders reject unknown keys; tags require a version bump or a lockstep optional field across Rust, web and native validators, and Roc, Rust and Zig SDKs.
2. **Span budget**. Style and token spans share 4096 (WP:1881-1887); tag spans would need their own budget or a share of it.
3. **Semantic cell is wider than a screen cell**. Board squares are 3 cells, invaders 2, the player 3, and several boards interleave spaces (section 4). Tags must repeat across covered cells (as tokens do) or the model needs a cell-group notion.
4. **Wide glyph continuations**. Rust requires continuation `token` to equal the head's (WP:2024, WP:2350); the web validators do not compare `token` (WS:743-752, WS:792-803). Tags would inherit this rule and this divergence.
5. **Patches carry whole cells** (section 1.4). Tags inside `ScreenCell` patch for free; tags elsewhere do not.
6. **Glyph tier has no 1-bit mode** and the token 1-bit column is unimplemented (section 1.5). Checkers and chess distinguish cursor from selection only by colour (fg 226 vs 46), which a monochrome client would lose; a flag gives the ASCII table a non-colour cue.
7. **`fg` 0 means default**, so true black is index 16, not 0 (WP:1633-1634); a `sys.color.black` mapping must avoid 0.
8. **`world_glyphs` keys are free strings** and glyph overrides may exceed one grapheme or two columns (section 3).
9. **Cursor overwrites content** in territory (248-252): the owner under a cursor is not in the frame. A `cursor` flag on the underlying cell would keep it.
10. **No viewer identity in `view`**, so viewer-relative flags and per-viewer identity visibility (record §4.1 "owner-visible") cannot be authored by world code today.
11. **Token is unused** by all worlds and dropped by two of three renderers; tag adoption should not assume the token path is exercised.

## Acceptance

Mapping to the record's slice 1 acceptance ("Checked-in inventory: buffer schema, tier list, per-world tag needs; v1 vocabulary candidates with ASCII-fallback sketches"):

- [x] Buffer schema: cell fields and widths, both wire shapes, limits, versioning, patches, tokens, other surface kinds, and the extension-point finding (sections 1.1 to 1.6).
- [x] Tier list: #786 tiers and display modes as coded, other clients, 1-bit status (section 2).
- [x] #5970 fallback chain as coded, with the free-string `world_glyphs` keys (section 3).
- [x] Per-world tag needs, one row per world, trivial examples grouped (section 4).
- [x] v1 vocabulary candidates (kind, flags, `sys.*`, `world.*`) with ASCII-fallback sketches, labelled as proposals with the record's defaults and Drew as decider (section 5).
- [x] Gaps and risks for the spec slice (section 6).
- [ ] After merge, link this record from #15244 and #6010.

Next: slice 2, tagged-state spec v1 (record §5 row 2): cell schema and encoding choice given section 1.3, closed kind set, flag bitmask, namespace spelling, vocabulary v1 with a complete per-tier fallback row per tag, and the extension rule, after Drew decides record §8 items 1 to 3.
