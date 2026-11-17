Fixes #7937

## What

PR #3855 wired `?done=all` only into the full-page `/board` route, so the board's own live-refresh fragment silently reverted a show-all Done view to the 100-card cap. Following the #8521 pattern for saved views:

- `server.ts` — the `/frag/board` handler now passes `{ showAllDone: url.searchParams.get("done") === "all" }` to `renderBoardBody`, exactly like `/board`.
- `board-view.ts` — `viewParamsQuery` also carries the `done` param, so `fragBoardUrl`'s refresh URLs (`data-on-load` / `data-on-dashrefresh__window`) preserve `done=all` across refresh cycles instead of dropping it.
- Default behaviour is unchanged: with no `done` parameter, `/board` and `/frag/board` still render at most 100 done cards with the "show all N" link.

## Tests

`bun test test/board-view-8521.test.ts` on the laptop: all 9 tests pass, including:

- updated #8521 expectations (done is now carried as view state, not dropped; unrelated params like `full=1` still are);
- new: `fragBoardUrl(?done=all)` → `/frag/board?done=all`;
- new handler-wiring pin: the fragment handler renders show-all Done under `?done=all`.

`test/author-picker-1179.test.ts` (the suite the issue suggested) fails identically on pristine main in this environment — its beforeAll spawns a server subprocess that times out (5s hook) before any test runs; pre-existing and unrelated to this diff (verified by running it in the clean primary clone).

Queue task 14554, lane any-b.
