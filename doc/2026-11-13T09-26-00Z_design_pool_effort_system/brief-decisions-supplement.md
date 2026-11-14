# Supplement to the pool & effort brief: the decision-model system

Drew, 2026-11-13 05:42 CST, supplementing the 2026-11-13 03:26 brief (`brief.md` in this folder). Transcribed faithfully and sectioned. This is part of the same design-mill program (starter issue code#9399).

## 0. "clef jev" — decoded

Clef and Jev are **not regular LLM models**. They are decision models, and the schema is known as **System 1 decisions**:

- **Jev** is Command Code's decision model, served on the `v1/systemone` contract: JSON state + typed questions in, **probabilities out, never text**.
- **Clef** fronts that same shape for OpenChamber's classifier client.
- Existing anchors in the fork: the façade program **opencode#1146** (one endpoint per real standard — chat-shaped traffic gets `/v1/chat/completions` + `/v1/responses`; the decision family keeps a systemone-standard endpoint) and the clef fold-in **opencode#1410**.

## 1. Endpoint support

- OpenCode should support the System 1 standard if it doesn't already: all conversation/etc. endpoints are accepted coming in and either **pass through** or **adapt** to the appropriate end state.
- If one protocol is always better when supported — or a smarter protocol is equivalent for interaction but more wire-efficient, or lets us add metadata for better performance or quality — prefer it. Support long-term caching of all sessions; conversations continuable by id, search id, tags, create/modify date, etc.
- Accept all model endpoints for raw usage of pools and providers; pass through or transform into a better protocol when there is no downside or a worthwhile upside.

## 2. Decision pools

Pool shapes (Drew gave two taxonomies; the mill settles the final shape):

- decision plan · decision LLM plan · decision free · decision LLM free · decision pay-as-you-go · decision LLM pay-as-you-go
- or: decision bland · decision free · decision pay-as-you-go — composed of decision native free, decision LLM free, etc.

The parts build up together into **the decision pool**, which is the one used by default because config sets it as the default decision model.

- Add **free decision models from every place we can find them** first, then all the appropriate **plan** decision models, then PAYG.
- Where more than one candidate exists, compare cost and capability.
- Track the **$10 (or whatever) of Cloudflare usage included with Workers** and use up to that included amount for decision calls, disengaging once the included amount is hit and pivoting after that.
- Add the **OpenAI** decision option **if it is cheap enough when converted into dollars of quota percentages**.
- Per provider, the mill must answer with data: is a native decision model worth using for decisions, or are the alternatives cheap enough that asking a cheap LLM (DeepSeek Flash or something) is the better buy?
- Possibly integrate **local decision models** if they exist and are used.

## 3. The executor

- Make a **skill or runbook or mcpx tool** for executing a decision. It takes in a JSON decision supporting either of the two or three main formats and returns valid JSON matching the schema, with valid percentages / the response to the decision.
- Config should set the decision-model default to our decision pool.
- **Every decision gets an ID.** Caller and called can each provide **points correct / points wrong, 0 to 5, on separate gauges**.
- Decision effort: allow any effort to be set on decisions, but by default it has no impact and is not used (see §6).

## 4. LLM-as-decider protocol

- **Don't use `small`.** Only Luna small / **Luna 100k** as the last-resort decision model. Maybe there is a super-fast free one that doesn't need to be smart at all — Beam might be too slow _and_ too smart; consider Beam or one of the really, really fast ones that are just not decision-API fast.
- An LLM decider uses **up to its context limit without compaction, or less when possible**, to review the input and possibly do a few quick scans of files, then produces valid JSON matching the schema.
- **Invalid JSON handling:** feed back the invalid JSON, the decision schema, the prompt, and suggestions; the result must be valid JSON **after two tries**. Then move on to the next LLM or pool; if all pool routes are exhausted, **set a default answer**.

## 5. Feedback and outcome capture (the learning loop)

Later we compare model outcomes to cost, speed, etc. Capture, per decision and per linked work item:

- PRs merged · issues closed/resolved · issues made · canceled · PRs closed incomplete · duplicate work · work not pushed · PRs and commits reverted · rework · work undone · caused incident · data loss · invalid · unclear · misunderstood · too-big · overcomplicated · oversimplified · perfect · simple · safe · fixed · partial · cheaper than expectation · unneeded-worry · overtested · undertested · significant expense · slow · heavy · network · not-scalable · etc.
- **Every session everywhere must know it can report**: "this issue or that PR got undone", "this issue was misunderstood and caused this PR, which blew a quota and caused perf issues, then an outage" — for every issue and PR.
- Linkage: each issue/PR is linked to a **task**; the task is linked to **one or more sessions and attempts** — various subsessions, tool calls, token uses, pools and models hit/failed/used; tokens in, tokens cached, tokens context, user input, thinking, tool calls and reads; **TTFT, total step time, first TTFT, total step count and time for every session and input**; number of question prompts, number of questions, time until submitted; errors auto-rerouted; **the decision model and the decision**, and the feedback from PRs and issues, parent sessions and subsessions.

## 6. What "effort" means for a decision model

Effort overlaps two related but technically distinct concepts:

1. **Tie-break lean** — when using a decision API to determine a routing direction: low/medium/high add a _very slight_ boost toward the lowest / closest to the middle (workhorse/scout) / closest to specialist-and-summit. Not an absolute amount, not a plain multiplication. An appropriate mechanism slightly prefers one direction, e.g. when the two highest results are within some distance of each other. Low/medium/high might also change **which default is used when auto is used**.
2. **Fallback pool choice** — the pool to use if all decision models fail.

For now, especially for the UI, leave it this way to avoid new widgets. In the API and on tasks we can distinguish the two. Later refinement — low/medium/high for **quality, speed, and cost** as separate axes: if they all match, their ratio is identical, so what does changing low→high mean? Working answer to test: choose the best all-around at a given overall capability band and pass to the appropriate pool without an effort level. Consider extensions supporting such steering for other decision types, dynamically at runtime, or both.

**Dependent probabilities:** a short setup context may hold a decision between 3+ probabilities and another with 2; probabilities may change depending on the first question, or the combination of Q1+Q3 determines Q4's value; Q2+Q4 determine or highly influence Q5.

## 7. Conditional clauses — an extension layer ABOVE the endpoint

On top of the System 1 API, support automatic if-this-then-that. Every question submitted to the decision model can carry functions and baked-in clause types:

- If 1, 2, 3 are yes/no/yes → switch Q4 to no. If yes/no/no → Q4 is yes but Q5 is no. If yes/yes/yes → Q4 is "1 and 2", Q5 is yes, Q6 is both.
- `if blank then this is X`; `if this and this and this then slight preference / strong preference for or against each of them`; optional rankings and other subclasses.
- If this is for 1 and that is for 2, then for Q4, if 3 is yes, strongly boost checking options B and C.
- If deferred or not answered → prefer checking just option 1. If no, or a number greater than 100 → check all the boxes and insert into the make-your-own.

Implementation shape: a **template string with access to variables** — all dependent questions, answers, and so on. This should **not** live in the systemone endpoint directly unless it can be done as a superset that does not affect the real standard in any way.

## 8. PM-stack review surface

Build review into the PM stack:

- Review worked items; provide **feedback, labels, and scores** for one item, a model-selected set of items, a set marked as a set, custom selections within a set, multiple sets, or each item standalone.
- Search and review **terminal-status items by epic, roadmap, and timeline**; add labels.
- Review **queued work**, reprioritize it, add labels.

## 9. The decision record

Track the decision LLM's **prompt with a version and a date**; make clear which prompt was used for **every** decision. Per decision, record:

- tokens used · percentages · quota (maybe) · time to decision
- whether the decision resulted in a successful session goal — from the **sub-session's own perspective**, the **parent's perspective**, or **anyone's perspective**, through association with an issue, PR, or epic that was provided a rating
- the invalid-JSON path (§4) and which pool/model finally answered

## 10. Cycle-safe pool traversal

Support any number of cycles in the priority of pools and plans accessed. If a pool has already been seen, it has already been processed through **all** available options — weighted, round-robin, and ordered, for the entirety of the pool — so it can be **safely skipped**. This prevents cycles and unnecessary error responses from providers we already know are walled or would reject us. Later, mutual references might be supported using something like the **NixOS module fixpoint** concept, or another method.
