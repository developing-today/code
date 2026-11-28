# Zero-context reader prompt

Give this to a fresh subagent before publishing any issue or dispatching a long brief, with the draft and nothing else; a harness that loads repository rules into every agent (for example `CLAUDE.md`) should not load them here. Fix every gap it reports, then publish.

```text
You know nothing except the document below. Use no other context. You may follow the document's own links to check that they resolve and say what they claim.

Document:
<the full draft body>

Return:
1. Restatement: in your own words, the goal, your role, and the first three actions you would take.
2. Questions: every question you would have to ask before starting, most blocking first.
3. Unlocatable claims: claims you could not find support for in the document or its links.
4. Broken or local-only references: links that do not resolve, and paths that exist only on the author's machine without saying what they contain.
5. Risks: places where two readings are possible and would lead to different work.
Keep it short and concrete. Your value is in what is missing, not in praise.
```
