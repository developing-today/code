# Running questions — final design defaults

Continuation of the commission/questions log on #6020. None blocks authoring.

| Question | Chosen default |
|---|---|
| Exact S39–S40 wording is not in this checkout or the program's linked comment; may the verified #6020 paraphrase be used? | Yes, explicitly label it a paraphrase, retain the missing-original limitation, and do not invent verbatim statements. |
| Does a landed v1 design or closed issue certify deployment? | No; require repository and runtime evidence for each integrated capability. |
| How do the old four logical sensitivity labels map to three physical tiers? | No silent alias; obtain an explicit migration contract and tests. Until then use only a proven restricted scope. |
| Can complete history, replay or alternative occurrences be universally exposed? | No; completeness is ingested, retained and currently authorized. Forbidden existence is not disclosed; revoked/deleted bytes resolve to permitted unavailability. |
| May usefulness or budget pressure authorize deletion or loosen classification? | No; routine retention, human-approved usefulness deletion and independently required rights purge retain separate authorities. |
| Should cold history silently join ordinary search? | No; use the later cold-format contract: explicit, authorized and budgeted addressing only. |
| Does a checksum or attestation authenticate truth, writer authority or a license? | No; integrity, signer verification, authority, permissions and rights are separate evidence. |
| Which learner, graph engine, placement algorithm and performance thresholds are selected? | None prematurely; keep the deterministic baseline, compare options and predeclare measured trial gates. |
| Which feedback/trace/receipt data may be retained or exported? | Minimize and restrict pending purpose, consent, rights, retention and downstream deletion decisions; no implicit training permission. |
| How should selected-shard failures interact with optional partial expansions? | Any selected-shard failure/timeout/cancellation fails the whole search; only independently optional expansion can be incomplete. |
| Enqueue does not accept the mandated `--worker repo-worker-code`; should the worker flag be omitted? | No. The installed `fleet queue enqueue /dev/null --queue repo-health-code-pilot --worker repo-worker-code --dry-run` rejected `--worker` before any enqueue. Honor the literal worker constraint. File the bounded follow-ons with a deliberate-not-queued comment and their complete triage packet in the same pass, as the repository filing rule permits. Do not alter fleet tooling in this design PR. Ownership verbs (claim/heartbeat/complete) all keep the mandated worker. |

The last constraint is an operational coverage gap, not a reason to block the
design. A later authorized caller or the standing coverage reconciler can enqueue
the issue packets through a compatible attributed surface. Do not claim those
follow-ons are queued until a successful live queue result exists.
