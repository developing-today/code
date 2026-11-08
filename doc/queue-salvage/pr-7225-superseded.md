## Superseded by main — closing with the salvage diff recorded

Shepherded by queue task 13533 (openchamber scheduled lane). Fresh state, 2026-11-08:

- `origin/main` = `e0ece4194e`; this PR's base is that same commit, `mergeable=MERGEABLE`, no `held` / `user/owned` / `do-not-merge` labels.
- Merged `origin/main` into `queue/5653-3179-jsonc-comments` (merge commits `46d23787db`, `ae526c16a5`).
- **Post-merge delta vs `origin/main`: zero lines.** `git diff origin/main origin/queue/5653-3179-jsonc-comments` is empty.

### Salvage diff: nothing unique remains

Every change this branch carried has already landed on main, in the newer canonical **v2** direction:

| This branch's change | Where it lives on main |
|---|---|
| String-aware `jsonc_to_json` scanner (inline `//`, `/* */`, trailing `,}`/`,]`, string lookalikes) | `pkgs/opencode-config-normalize/normalize.sh` — present verbatim |
| Unterminated `/*` rejected instead of silently truncating the file | present verbatim |
| Comment/string-aware live-key locator for the plugins rename | present as `find_top_level_key` + `find_value_span` (the #4312 review fix) |
| Fixtures 7–11: inline-comments, block-comments, string-lookalikes, unterminated-comment, comment-plugin-key | `normalize_test.sh` cases 7–11, retargeted to v1→v2 |
| `plugins` → `plugin` rename block | **superseded**: main normalises the other way (v1 → canonical v2 `plugins`/`agents`), per `0baa24fd83` and the #4639/#4612/#4770 follow-ups |

The only thing the branch had that main does not is the v2→v1 rename direction, which main deliberately reversed. Keeping it would flap the config keys against the normaliser, so it is dropped rather than salvaged.

### Verification

- `bash pkgs/opencode-config-normalize/normalize_test.sh` on the merged tree: **all cases passed** (including `inline-comments`, `block-comments`, `string-lookalikes`, `unterminated-comment`, `comment-plugin-key`, `block-comment-inner-delimiter`, `mode-preserved`, `symlink-refused`).
- `bash -n` clean on `normalize.sh` and `normalize_test.sh`.

Closing as superseded. Branch left in place; it will go with the PR's own close flow — no branch was deleted outside that flow.
