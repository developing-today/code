# Fleet

Fleet is one multicall binary for the fleet's tools: a single executable that
behaves as many different programs depending on the name it is invoked under
and the first argument it is given. Invoked as `fleet`, it dispatches to one
of twelve domains — the work queue, secrets, placement and remote dispatch,
the session proxy, the message bridge, agent enrolment, search, the GitHub
passthrough, shell access, the laptop-agent client, and the dispatch clients.
Invoked under an alias name such as `repo-queue` or `secrets-broker`, it
routes straight into the corresponding domain. There is one binary to build,
install and keep current on every machine the fleet runs on.

Fleet exists because each fleet tool used to re-solve the same four problems:
a shared work queue where work is claimed by exactly one worker under a
time-limited lease; secrets resolved by name from a resident broker instead
of a 10–40 second vault call per lookup; remote build placement decided from
live capacity data across GitHub Actions copies of the fleet repositories;
and agent coordination — a mailbox, enrolment, a health check, and one rule
(the intake ladder) for what an agent works on next.

## Current state (2026-11-23)

- The queue is the project's execution substrate and its correctness work is
  merged-list item 1: in flight and substantially landed. The repair verb for
  quarantined packets shipped (PR #13218) and a production drain of 37 repairs
  ran on broker generation 6; the double-serve defect (#13091) is fixed; the
  fleet book's roadmap puts reconciling the production repair cohort first,
  ahead of any queue-policy change.
- The fleet-static rollout completed on 2026-11-23: the static binary was
  published, the laptop queue broker flipped to generation 6 (generation 5
  kept as rollback), and placement criterion 1 was proven across seats
  (#13146, closed).
- Any enrolled agent, of any kind, can claim any queue job under any name —
  the lane ruling implemented 2026-11-22 (#12544, PR #12558) and verified live.
- The quota plane's staged acceptance is in flight (item 28): observation
  history is accumulating, with the first seven-day interval elapsing
  2026-11-30.

## Where its detail lives

- Book: the [Fleet book](https://developing-today.github.io/code/fleet/) —
  sources at `pkgs/fleet/book/`, including its own programme roadmap and
  issue map (190 open fleet issues in its 2026-11-23 census).
- This programme's slice: the fleet book's [high-level
  roadmap](https://developing-today.github.io/code/fleet/development/high-level-roadmap.html)
  carries fleet's share of the merged list (items 1, 2, 3 and 28 in full).
- Programme tracker: [#13272](https://github.com/developing-today/code/issues/13272).
- Merged-list items: 1, 2, 3 and 28 are fleet-primary; fleet also carries
  cross-links to items 6, 7, 9, 14, 17, 19, 25, 26, 27, 29 and 30.
