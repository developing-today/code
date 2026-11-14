# Brief: prompt & chat surfaces across the PM stack

Drew, 2026-11-13 05:54 CST. Transcribed faithfully and sectioned. Goal: every appropriate PM-stack page carries prompt widgets, and the PM stack gains a general chat/session surface backed by the same machinery.

## 1. Prompt widgets, by scope

Generate pages and widgets on all the appropriate PM-stack pages to provide a prompt for:

- **a given section** of a page
- **a page overall**
- **the PM stack generically**
- **the PM stack generically, just to access an agent** — chat and task delegation, queued at a priority level
- **specialized task variants**: reprioritization, detail gathering, comparing new options, integrating new insights and developments since last review, etc.

## 2. Targets and routing

- Set **any prompt, and the default prompt target, to Claude or Muse**.
- For each of the main OpenCode/OpenChamber pools (low, medium, high), support **auto with low, medium, and high** where that exists.
- Possibly support a **variant of our extension of the decision model** (see the decision supplement in the pool/effort mill, code#9399): it takes input and **generates a series of questions**, passed to a decision API or an LLM or a hybrid of both, with **effort steering and default fallbacks**; it allows answering **"not confident"** for any answer, uses **speed-optimized models**, and generates a set of answers to the questions with a **provided context level**.
- Methods to **access the advanced constraints** (the conditional-clause machinery from the decision supplement).
- Methods to take the question and use **any given model or queued task to generate and provide the appropriate context**, along with the prepared question **in the appropriate shape**, to the decision API.
- Allow **uploading attachments and processing multimedia**.
- Consider a future enhancement: **streaming audio, one or both ways**.

## 3. Async and sync modes

- **Async requests**: may give a post notification on completion, or not; provide an **ID that can be checked on**.
- **Sync versions** that do the same, or run immediately and either: don't return until answered, or **open a WebSocket or other channel** until the answer to the prompt is provided.

## 4. What Drew will use it for

- Ask any questions he might ask in chat
- Suggest new ideas or design mills
- Reprioritize certain work
- Authorize specific kinds of bug fixes or major decisions
- Ask questions about recent work, including requesting generation of a report, or a refresh/view of specific data or news
- Generate or update appropriate pages in the PM stack

## 5. The chat/session surface

- **Allow reviewing all chat messages.** Generate a **new chat or session for each message sent**, unless responding to a session item or response, or providing input on a report page.
- Something like a **T3 or Durable-object/pi variant or generic interface**, with a **notification count and push notifications**.
- Ability to: **open and close chats · access archived chats · reply to messages · reference other chats within a new message or a message in another chat** · and so on.
- Possibly expanding later to support — or be equivalent to — posting in **Slack or Telegram**, or directly to **GPT, Claude Duet, OpenClaw, Muse, Grokbot**, or other persistent agents or session providers.
- **Defaults**: typically get an **immediate response from a Muse hook task**, or **queue an item with medium-to-high priority using auto or workhorse**; or possibly get picked up by Muse because it isn't filtered/specified. **Anything that is a default can be overridden as needed.**
