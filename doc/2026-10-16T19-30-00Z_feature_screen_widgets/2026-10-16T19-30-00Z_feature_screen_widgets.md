# Screen widgets: `fill` and `rule`

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
- **Text helpers deferred.** The proposed `RocText` module is not shipped: the
  pinned Roc nightly is the reason apps avoid some string APIs, and a text
  library would have to be validated against that backend first. `RocJson` and
  `RocCap` already own the string-to-structure work apps need most.

## Acceptance

- `screen_widgets_fill_and_rule` compiles a small app that fills a 3x2 block,
  draws a six-column rule, and writes a number, then runs it in the sandbox and
  asserts the exact plain-text rendering:
  `"xxx   \nxxx 7 \n------"`.
- All checked-in apps still build.

## Follow-ups

- A table/list widget once a second app needs the same layout.
- `RocText` after a Roc nightly bump, with the same compile-and-run coverage.
