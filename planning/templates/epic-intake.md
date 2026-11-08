# Epic intake checklist

Use this checklist whenever a new multi-issue program is filed.

1. Start from the [Epic issue template](../../.github/ISSUE_TEMPLATE/epic.md). Give the epic a named owner and a measurable definition of done.
2. Keep the template's `epic` and `kind/feature` labels. Keep the canonical `app/<component>` label for the work; replace the default `app/pm-stack` when the epic belongs to another component. Add the usual source, priority, effort, and mode labels for the actual work.
3. Add the issue to org Project #4 and set **Area**, **Horizon**, and **Kind = Epic** there. A value written in the issue body does not set a Project field.
4. File each child issue with its own acceptance criteria. In the same intake pass, attach it to the epic using GitHub's native **Create sub-issue** action. Add `Part of epic: #<epic-number>` to the child body as the human-readable mirror; prose links alone are not a hierarchy.
5. Re-read the epic and every child after filing. Confirm the labels, Project #4 fields, native parent/child links, owner, and definition of done.
6. Run the [epic conformance audit](../../pm-stack/aggregator/scripts/check-epic-conformance.ts) from `pm-stack/aggregator/`. Resolve each reported gap or record an explicit exclusion on the epic.

The audit checks the GitHub issue record and, when `gh` has `read:project`, Project #4 membership. If that scope is unavailable it reports the omission rather than treating an unreadable project list as empty. Verify the actual Area, Horizon, and Kind values in the Project UI; a project membership alone does not prove those fields are correct.
