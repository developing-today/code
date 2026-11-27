# OpenAI plan-math review — #7013 A1

Checked: 2026-11-27
Disposition: **No model-seat or metering change required.** The explanatory catalog comment was clarified to distinguish the account's eligible-model set from this alias's actual model mapping.

## Lifecycle evidence

OpenAI's [API deprecations page](https://developers.openai.com/api/docs/deprecations), checked 2026-11-27, lists:

- `gpt-5` and its dated snapshot for shutdown on 2026-12-11; replacement `gpt-6.1-sol`.
- `gpt-5.3-codex` for shutdown on 2027-04-01; replacement `gpt-6-sol`.

The dates and replacements recorded in `doc/model-deprecation-watch.md` §3 D4–D5 are consistent with the current page.

## Fleet exposure and bucket mapping

- In `.opencode/opencode.jsonc`, `gpt-5.3-codex` occurs only as a model definition in the `_commandcode` catalog. A search of the config's route and pool targets found no `_commandcode/gpt-5.3-codex` seat. It therefore has no current fleet route exposure to migrate.
- The `openai-payg` catalog has the GPT-6.2 and GPT-6 models; the `openai-payg-free` alias intentionally declares only `gpt-6-luna`, `gpt-6-astra`, and `gpt-6-sol`. Its `tokenMetering.groups.large.models` whitelist is exactly those three IDs, with a 1,000,000-token daily bucket and 20,000-token margin. No retiring GPT-5 ID is mapped to the free alias.
- OpenAI's [complimentary-token program article](https://help.openai.com/en/articles/10306912-sharing-feedback-evaluation-and-fine-tuning-data-and-api-inputs-and-outputs-with-openai), checked 2026-11-27, lists the GPT-6 models in the large group alongside separately versioned/retiring model IDs. It confirms the group is shared and daily; the list's shutdowns do not change this alias's explicit GPT-6-only mapping. It also says OpenAI will give 30 days' notice before terminating the complimentary-token program.

Therefore the `openai-payg-free` daily bucket mapping remains valid after the cited GPT-5 shutdowns. The existing 1M/20K metering configuration and its three-model whitelist are unchanged. The nearby comment in `.opencode/opencode.jsonc` now points readers to the dated provider eligibility list and avoids presenting a shutdown-sensitive GPT-5 family list as this alias's catalog.

## Verification

- Config search: `gpt-5.3-codex` has one occurrence, its `_commandcode` catalog definition; no corresponding route target.
- Config inspection: `openai-payg-free.models` and `tokenMetering.groups.large.models` both contain exactly `gpt-6-luna`, `gpt-6-astra`, and `gpt-6-sol`.
- No runtime model, route, or metering values changed; only the explanatory comment changed.

This is the linked review record for `doc/model-deprecation-watch.md` §4 A1 and issue [#7013](https://github.com/developing-today/code/issues/7013).
