# gh-post content runtime

`content-ledger/` and `content-throttle/` are imported unchanged from
`dezren39/hatch` (`tools/`), as tracked at commit
`c11f41086f57d626be7eb582763b721f8f557408`. The latest source change to
these directories is `153d81720156802d2d20653abafeb37ae9a54f22`
(`content-throttle: pacing wrapper (warn-only), circuit breaker, gap detector;
gh-post gate + live note (hatch#13)`). This restores the adjacent runtime
dependencies omitted when `gh-post` moved here in code PR #12910.

Design: code #2013 / PR #2226; implementation tracker: code #2227;
source implementation: hatch #3 (ledger) and #13 (throttle).

This slice does **not** enable enforcement or establish that fleet-wide
deployment, the warn-week review, monitor wiring, or the post-enforcement
success measurements are complete. Fresh throttle state still defaults to
`warn`; existing stored mode and environment overrides remain unchanged.
See `content-throttle/README.md` for the original rollout contract; its
`tools/` commands refer to the source layout (here use `pkgs/gh-fleet/`).

## Focused checks

All checks use the standard library, temporary state, and stub GitHub calls;
none should post to GitHub. Oracles: repository-relative module paths for
the import regression; stub object readback and reservation-before-call for
the ledger; fake-clock budget/breaker invariants and stub readback for the
throttle suite.

```sh
python3 -I pkgs/gh-fleet/test-gh-post-runtime.py
python3 -I pkgs/gh-fleet/content-ledger/test_content_ledger.py
python3 -I pkgs/gh-fleet/content-throttle/test_content_throttle.py
python3 -I pkgs/gh-fleet/test-gh-post-app-leg.py
```
