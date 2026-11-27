# Model deprecation watch — sources registry + seed records

Rebuilt 2026-11-24 (developing-today/code#14236, follow-on of #7013; first sweep task 22909).
Prior v1 artifacts (tasks 11448/18517, goal-workspace `hidden_files`) were unverifiable
on every reachable surface, so this file is the registry of record. It lives in the
repo because the goal workspace proved non-durable.

Cadence: weekly, Mondays 09:12 America/Chicago (`model-deprecation-watch-weekly.timer`,
Surface 2b), plus event-driven runs on flagship launches. Registry row in
`doc/fleet-schedule-registry.md` is maintained in the same pass as any schedule change.

## 1. Sources registry

| Provider  | Announcement / lifecycle surface                                                                                   | Checked 2026-11-24 | Notes                                                                                                                        |
| --------- | ------------------------------------------------------------------------------------------------------------------ | ------------------ | ---------------------------------------------------------------------------------------------------------------------------- |
| Anthropic | https://platform.claude.com/docs/en/about-claude/model-deprecations                                                | live               | Full status table + dated history; ≥60-day notice commitment; partner clouds (Bedrock, Google-managed) set own dates         |
| Anthropic | https://platform.claude.com/docs/en/about-claude/models/overview                                                   | live               | Current lineup + per-model retirement floor                                                                                  |
| OpenAI    | https://platform.openai.com/docs/deprecations                                                                      | live               | Upcoming + past tables with shutdown/notice dates; `shutdown_date` also exposed via `GET /v1/models` for programmatic checks |
| Google    | https://ai.google.dev/gemini-api/docs/models + https://ai.google.dev/gemini-api/docs/deprecations                  | live (models page) | Deprecation callouts inline on models page; `latest`-alias hot-swaps get 2-week email notice                                 |
| DeepSeek  | https://api-docs.deepseek.com/quick_start/pricing                                                                  | live               | No dated table; retirements noted inline (legacy names still accepted, served by successor)                                  |
| Mistral   | https://docs.mistral.ai/getting-started/models/models_overview + https://docs.mistral.ai/inference/model-lifecycle | live (overview)    | Full deprecated/retired table with deprecation→retirement windows                                                            |
| Meta      | none found                                                                                                         | gap (unchanged)    | No Llama lifecycle surface identified; gap flagged as before                                                                 |

## 2. Seed records (from #7013 design, Drew 2026-10-21)

### S1. Claude Haiku 4.5 — consumer-surface deprecation

- State 2026-11-24: `claude-haiku-4-5-20251001` is **Active** on the Claude API with
  retirement "not sooner than October 15, 2026" — i.e. past its floor; retireable
  at any time. The original seed concern (deprecated everywhere except
  business/enterprise tiers and some API providers) is consistent with Anthropic's
  partner-cloud carve-outs (Bedrock / Google-managed set own dates).
- Fleet exposure: no Haiku 4.5 pin found in fleet route/pool config; Claude legs
  ride current-generation aliases. Exposure: none.
- Action: none. Re-check weekly; escalate to plan-math review if a dated
  deprecation lands.

### S2. ChatGPT models — consumer-surface deprecation, API survival

- State 2026-11-24: OpenAI's table confirms the pattern at scale — chat variants
  (`gpt-5.1-chat-latest`, `gpt-5-chat-latest`) already shut down Jul 23 2026, while
  API snapshots persist to their shutdown dates. Current upcoming: `gpt-5` family
  shutdown **Dec 11 2026**; GPT-4o / 4.1 snapshots **Jan 14 2027**.
- Fleet exposure: fleet OpenAI catalog still defines `gpt-5.4`, `gpt-5.5`,
  `gpt-5.6-*`, and **`gpt-5.3-codex` (shutdown Apr 1 2027, replacement `gpt-6-sol`)**.
  → plan-math review action filed with this sweep (see §4, A1).
- Action: A1 open.

## 3. First-sweep deltas (2026-11-24, all surfaces fetched live)

| #   | Finding                                                                                                                | Fleet relevance                                                         | Disposition                                           |
| --- | ---------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- | ----------------------------------------------------- |
| D1  | Anthropic `claude-opus-4-5-20251101` deprecated **2026-11-23** (yesterday), retirement Jan 22 2027 → `claude-opus-5-5` | No 4.5 pin in fleet config                                              | Note only                                             |
| D2  | Anthropic `claude-sonnet-4-5-20250929` retires **Nov 30 2026** (6 days) → `claude-sonnet-5-5`                          | No 4.5 pin in fleet config                                              | Note only; confirms 4.x prunes are in flight          |
| D3  | Anthropic Haiku 4.5 past its retirement floor (Oct 15 2026) but still Active                                           | Seed S1 unchanged                                                       | Keep on weekly watch                                  |
| D4  | OpenAI `gpt-5` / mini / nano / pro shutdown **Dec 11 2026**                                                            | Catalog bucket comments reference gpt-5 family for free-tier buckets    | Verify bucket mapping still valid after shutdown (A1) |
| D5  | OpenAI `gpt-5.3-codex` shutdown **Apr 1 2027** → `gpt-6-sol`                                                           | Seated in OpenAI catalog definitions                                    | Plan-math review (A1)                                 |
| D6  | Google `gemini-2.5-flash` / `gemini-2.5-pro` auto-route **Dec 1 2026** (7 days) → 3.5-flash-lite / 3.8-flash           | Fleet already unseated 2.5 line (404 for new users, per config comment) | Confirms existing seating; no action                  |
| D7  | Google Imagen 4 shut down; several 2.0/3.x previews shut down                                                          | No fleet use                                                            | Note only                                             |
| D8  | DeepSeek legacy `deepseek-v4-flash` names retired, served by V4.1-Flash-1017 at Flash price                            | No fleet DeepSeek pin checked                                           | Note only                                             |
| D9  | Mistral deprecated table (Medium 3.x, Large 3, Small 3.x, Devstral/Magistral 1.x lines)                                | No fleet Mistral pins                                                   | Note only                                             |

## 4. Plan-math review actions

- **A1 (open, from this sweep):** reconcile fleet OpenAI catalog seats against the
  Dec 11 2026 (`gpt-5` family) and Apr 1 2027 (`gpt-5.3-codex`) shutdowns; confirm
  free-tier bucket mapping in `openai-payg-free` comments still valid post-shutdown.
- Cadence rule: every future sweep that touches a model in a fleet pool/route/plan
  files the same kind of action here and links it on #7013.

## 5. Sweep log

- 2026-11-24 (task 22909): all six provider surfaces fetched live; seed records
  updated; D1–D9 recorded; A1 filed. Posted on #7013.
