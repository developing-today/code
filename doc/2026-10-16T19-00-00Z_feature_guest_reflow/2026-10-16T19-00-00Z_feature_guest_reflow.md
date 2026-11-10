# Guest-aware reflow: a `layout` entry point

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md), the [viewport and paste record](../2026-10-15T16-00-00Z_feature_world_viewport_and_paste/2026-10-15T16-00-00Z_feature_world_viewport_and_paste.md), and the [screen rendering record](../2026-10-16T14-00-00Z_feature_screen_render_alignment/2026-10-16T14-00-00Z_feature_screen_render_alignment.md).

## Intent

Structured screens were cropped per connection, so a phone-width terminal and a
wide browser saw the same fixed grid with different amounts cut off. Let a
program render for the connection's actual viewport while keeping cropping as
the compatibility path.

## Decisions

- **A required `layout` entry point.** The platform `requires` block now asks
  for `layout : model, U64, U64 -> Str` next to `view`. The host wrapper
  `plaza_layout(model, width, height)` and the Rust sandbox call it with the
  session's viewport. Programs that cannot adapt return `view(model)`; the
  checked-in apps do exactly that except `roc-screen`, which widens its box to
  30 columns on a viewport of at least 30.
- **Render-only, never journaled.** `layout` is called by the view path with
  the connection's viewport, after the event sequence is read. It changes no
  model state and advances no sequence, so resize stays session-local exactly
  as before. Replays never see it.
- **Each connection renders for itself.** A session with a viewport calls
  `view_at(token, width, height)` on the actor; without a viewport the fixed
  `view` path is used. Structured screens still pass through the validator's
  `fit` crop afterwards, which is a no-op when the program already reflowed and
  a safety net for programs that did not.
- **Compatibility note.** Adding a required entry point means every app must
  define `layout`. All checked-in apps and the `id world new` template do, and
  the compiler reports a missing one by name. External apps (none yet) would
  need a one-line `layout = |model, _w, _h| view(model)`.

## Acceptance

- `checked_in_roc_screen_reflows_for_a_wider_viewport` asserts the fixed view
  is 20 columns, `layout_at(40, 5)` is 30, `layout_at(24, 5)` is 20, and the
  fixed view is unchanged afterwards.
- Every checked-in app rebuilds with the new required function and all
  checked-in sandbox tests pass.
- `the_counter_source_compiles_installs_and_runs` and the other compile tests
  still pass with their updated inline apps.

## Follow-ups

- A `--viewport WxH` flag on `id world join` for testing reflow without a real
  terminal resize.
- Per-widget reflow guidance once apps share layout code.
