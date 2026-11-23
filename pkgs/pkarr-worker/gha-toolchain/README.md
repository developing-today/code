# Optional pkarr GitHub Actions publisher

This lock is scoped to the workflow's runner-temp release copy. It pins
cf 1.0.0-beta.32 and Wrangler 4.161.0, the minimum Wrangler that this
cf release supports. The pkarr project lock, Wrangler config and
`nix run .#pkarr-deploy` operator path remain unchanged.

The workflow is manual, canonical-repository/main-only and off unless
confirmed. The `pkarr-worker-production` environment must already have
one required-reviewer rule with self-review disabled, administrator bypass
disabled, and exactly one custom deployment rule for the branch `main`
(no tags). The detached preflight checks these before associating the
deployment job with the environment. Environment credential configuration and creation of
the existing Worker are outside this workflow.

Upload creates no deployment. Its receipt binds the exact version ID
returned in the pinned cf JSONL output file (not stdout) to the Worker/account, full source SHA, successful run
attempt, and prior serving deployment. Start promotion with those exact
values. If traffic changed, prepare a new upload and review the new
baseline. GitHub concurrency covers only this workflow. Pause the
separate operator publisher or any other deployment path for the
promotion window: the last baseline read and deployment write are not an
atomic compare-and-swap. Success requires the exact deployment ID returned
by the create command to be actively serving the candidate at 100%. If a provider write's result is uncertain, read
back the state before deciding whether to try again.

This workflow does not change routes, Durable Object migrations, Worker
triggers, or settings. The existing Nix path remains available for
bootstrap and operator recovery.
