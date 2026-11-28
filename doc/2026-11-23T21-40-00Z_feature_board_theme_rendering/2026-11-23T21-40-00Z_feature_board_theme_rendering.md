# Portable board theme rendering (web + terminal) — #6163

See [original task packet](https://github.com/developing-today/code/issues/6163).
Parent: #5525. Governing design: #2054. Settled contracts: #5858
(`doc/2026-11-10_design_id5858.md`), #5679 (markup & styling model,
closed 2026-11-23).

## Request

Implement #6163 now that the hard gate is settled (verified live
2026-11-23: #5858 CLOSED-completed, #5679 CLOSED-completed, #5850
CLOSED-completed). Scope from the issue: render the finite data-only
theme tokens/layout/assets in web and terminal clients; verify
content-addressed asset bytes; safe defaults for missing/unsupported
theme, asset, or layout; accessibility overrides and post/board/viewer
precedence consistent with #5858; prove theme content cannot execute
HTML, JavaScript, CSS, or terminal escapes. No theme marketplace, no
arbitrary styling language.

## Approach

No board clients exist yet (verified: no CLI board commands, no id-web
board pages, no `BoardView` consumers outside `compose_view`), so this
slice lands the shared render layer both clients will consume, in the
same projection discipline as `compose_view`:

- New module `pkgs/id/src/boards/render.rs`: pure per-token cascade
  (viewer accessibility > viewer preference > post > board > platform
  default), render-time value grammars per reserved role (ring 2),
  contrast/accessibility floor (ring 3), content-addressed asset
  verification, and two emitters — safe HTML (`render_web`) and safe
  terminal text (`render_terminal`).
- Schema splits from #5858 (split 1: `Post::theme_ref`,
  `Board::foreign_theme_mode`) are NOT in this slice; the cascade
  accepts an optional post-theme input and a local `ForeignThemeMode`
  so precedence is implemented and testable, defaulting to `Honor`.
- Invariants held: data-not-code (closed value grammars, closed layout
  enum, hash-only assets, no remote fetch), per-token fixed order,
  non-countermandable accessibility, unthemeable chrome (closed
  theme-settable role list), clamp-never-reject, web/terminal parity.

## Decisions

- Reserved theme-settable roles are a closed list aligned with the
  #5850 tone dimension plus `type-body-size`, `font-body`,
  `motion-duration`; anything else is custom data (ignored, never an
  error). Promoting a custom token is a vocabulary change, not a
  theme-author act.
- Colour grammar is strict hex (`#rgb`/`#rrggbb`) only; hostile values
  (`url(`, `<script>`, expressions) fail the grammar and drop one
  cascade layer with a label — they never reach an emitter.
- Terminal output maps validated hex through a closed hex→ANSI-256
  function and strips all C0/ESC/DEL controls from untrusted text.
- Asset bytes verify by recomputed SHA-256 against the referenced
  hash, plus magic-byte kind allowlist (image/font); anything else is
  omitted with an "asset unavailable" label. Ring-1 media-type gating
  at publish stays a follow-up with the schema split.

## Follow-ups (not this slice)

1. Schema split: `Post::theme_ref` + `Board::foreign_theme_mode`
   (#5858 split 1) with legacy-decode and round-trip tests.
2. Wire `render_web` into id-web board pages and `render_terminal`
   into the terminal/CLI board surface when those land.
3. Publish-time validation report tooling (#5858 ring 3 tooling).
