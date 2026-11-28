# Screen widgets and `RocText`

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md) and the [screen rendering record](../2026-10-16T14-00-00Z_feature_screen_render_alignment/2026-10-16T14-00-00Z_feature_screen_render_alignment.md).

## Intent

Apps hand-rolled rect fills and separator lines one `put_text` call at a time.
Two small `Screen` helpers cover the common cases.

## Decisions

- **`fill(screen, x, y, w, h, ch, style)`** paints a rectangle with one
  grapheme and one style, clipped by the existing `put_text` bounds.
- **`rule(screen, x, y, w, ch, style)`** is a one-row fill, for separators
  under a status line or between table sections.
- **No layout engine.** These are drawing primitives, not widgets with state;
  an app still composes its own layout. A table/list widget would need a data
  model and is deferred until two apps need the same one.
- **`RocText` ships with only proven operations.** `trim`, `lowercase`,
  `starts_with`, `drop_prefix`, `contains`, `lines`, and `join_lines` are
  implemented over the string operations the pinned Roc nightly builds
  reliably (`Str.to_utf8`/`from_utf8`/`split_on`/`join_with`/`contains`,
  `List.get/sublist/map`). Byte-level transforms turn invalid UTF-8 into `""`
  rather than crashing. `RocText` joins `Key`, `Screen`, `RocJson`, `RocGrid`,
  and `RocCap` as a grantable module name.

## Acceptance

- `screen_widgets_fill_and_rule` compiles a small app that fills a 3x2 block,
  draws a six-column rule, and writes a number, then runs it in the sandbox and
  asserts the exact plain-text rendering:
  `"xxx   \nxxx 7 \n------"`.
- `text_helpers_trim_case_split_and_prefix` compiles and runs an app that
  exercises every helper and asserts the exact rendering
  `"t=AbC|l=abc|p=yes|d=go|m=keep|j=a\nb\n|c=yes"`.
- All checked-in apps still build.

## Follow-ups

- A table/list widget once a second app needs the same layout.
- Unicode-aware casing and trimming after a Roc nightly bump; the current
  helpers are deliberately ASCII/byte-level.
