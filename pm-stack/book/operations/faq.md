# FAQ

Questions a new operator asks, answered from the surveys. Each answer names its source. Where the surveys record a disagreement, the answer cites it and follows the code.

## Is the notify relay live?

No (D5). The `pm-notify` Worker is recorded in `pm-stack/platform/workers.json` as "not deployed", with a null address. Its event-mirror route and its edge answer-intake routes are not routed in its code; they arrive with steps 2 and 3 of the relay build. The dead-man's switch — one `stack.silent` push when the laptop falls silent — does not exist in production. Phone answers go to the laptop aggregator. See [Tutorial: trace a push notification from event to answer](../tutorials/trace-a-push.md).

## Is Kaneo our fork?

No (D1). The deployed board is the upstream image `ghcr.io/usekaneo/kaneo:2.47.0`, pinned by digest in `pm-stack/compose.yml`. The fork at developing-today-forks/kaneo exists as a repo, and fleet references point at it. Compose does not run it. Program text that calls the deployed board the fork is ahead of the deployment code.

## Does the aggregator read Pad data?

It reads no Pad planning data (D3). No source adapter consumes Pad content. The aggregator does probe Pad for health and topology: it carries a Pad base URL in `config.ts`, a Pad service link, and its `/api/health` has reported Pad up on port 7777. The stack-token exchange also accepts `pad` as a target. The README sentence "the aggregator reads no Pad data" is broader than the code; the accurate form is: no planning data, health only.

## Who writes GitHub labels for needs-Drew items?

The console does, and the aggregator service does not (D4). The aggregator's census header states its own rule: the aggregator never writes labels back. The console, in the same stack, writes them through its action table: `handsDone()` posts a comment and deletes the `user/escalated` label through `gh`, and `Ops.setRank()` adds and removes rank labels. The orchestrator remains the single writer of Project _fields_; labels are the seam, and the console is a label writer.

## How many Cloudflare Workers are there?

Twenty in the platform registry, 22 in the programme census's estate scope (D6). `pm-stack/platform/workers.json` lists 20, and its test holds the list to the repository's wrangler files. The platform README's "eighteen" is stale. The census ledger counts 22 across its wider estate scope. Cite the count with its source.

## Does the PM stack call an LLM, or use embeddings?

The aggregator, console, dispatch, and projector make no LLM calls and no embedding calls. The in-tree inference is the voice line Worker's: Workers AI speech-to-text with `@cf/openai/whisper-large-v3-turbo` and text-to-speech with `@cf/myshell-ai/melotts`, with template-built answers between them and no LLM. The embedding model in the estate belongs to Atlas: `Snowflake/snowflake-arctic-embed-m-v2.0` at 768 dimensions. No PM component consumes Atlas embeddings in the surveyed code.

## Where do I answer a question?

On the surface that raised it. A single needs-Drew question: tap an option on its push, or use its answer page. A decision set: its page at `/decisions/&lt;setId&gt;`, or the console at `#needs/questions`. A census item: reply from the console, or use the hands-done action for a hands item. The full procedure is [Tutorial: answer a waiting question end to end](../tutorials/answer-a-question.md).

## What is an aggregator in the hierarchy?

Nothing (D10). "Aggregator" names the dashboard service, not a work-item type. Kind on Project #4 is exactly `Task` or `Epic`. The ladder is program → project → epic → task or item, carried by four different systems and joined by refs.

## Which roadmap is the roadmap?

There are two roadmap implementations (D12). The aggregator's `/roadmap` merges Grist rows, repo-file rows, and Project-pool data, with the repo file winning a title collision (#1738). The console's `#more/roadmap` is an independent implementation in `console/src/plan.ts`, with its own phase vocabulary (`doing`, `now`, `next`, `later`, `built`) derived from labels. Name the surface when you cite a roadmap state.

## Which horizon does an item have?

The horizon on the surface you read it from (D11). Repo-file and Grist roadmap rows use `Now`, `Next`, `Later`, `Ideas`, `Ongoing`. Aggregator tier items use `today`, `tomorrow`, `next-3-days`, `longer`, `incoming`. Project #4 has its own Horizon single-select, read verbatim. No surveyed document maps the three vocabularies onto each other.

## Is Grist pinned at v1.7.21?

UNVERIFIED (D2). The README says `gristlabs/grist-oss:v1.7.21`, pinned by digest. The deploy file, `pm-stack/grist/compose.yml`, names no tag at all — digest only. Whether that digest is v1.7.21 is unverified in the surveys.

## Which service worker owns my phone's push subscription?

UNVERIFIED from the code surveyed. Two service workers exist: the aggregator PWA's, generated from `aggregator/src/sw-source.ts`, and the console app's, in `console/public/sw.js`. The surveys do not settle which one owns a given phone's subscription in production. Read the subscription row in the alerts store for a concrete phone.

## Where does a single-question answer get written?

To the needs-Drew question store, then the ledger (D9). The endpoint, `POST /api/needs-drew/&lt;ledgerId&gt;/answer`, calls `recordAnswer` in `pm-stack/aggregator/src/needs-drew.ts` writes it to the needs-Drew question store, the file at `cfg.needsDrewFile`. That path is `PM_AGG_NEEDS_USER_FILE`, else the legacy `PM_AGG_NEEDS_DREW_FILE`, else `needs-user.json` (or the legacy `needs-drew.json` where only that exists) under `~/.local/state/pm-aggregator/`, resolved by `needs-user-state-path.ts`. Decision sets keep their own store, `decisions.json`.

## An item looks done. Is it done?

Check the guards before answering yes. An automation close reopens when sub-issues stay open or the closing pull request carried no closing keyword. An epic stays open while its subtree has open children. The checks and fixes are in [Troubleshooting](troubleshooting.md), and the item's full path is in [Tutorial: follow an item from issue to done](../tutorials/follow-an-item.md).

## Where do quota figures live?

In the LLM-plans chapters, with dates. The quota tutorial in this book, [Tutorial: read the quota and usage views](../tutorials/read-the-quota-view.md), describes the surfaces only. A figure without a date and a surface is not a reportable reading.

Sources. survey/disagreements.md D1–D12, D14; survey/models.md §1–§2; survey/push.md §1; survey/website-pages.md §F; coverage-ledger.md Table A. Ground truth: laptop clone at 4d937b3, 2026-11-23.
