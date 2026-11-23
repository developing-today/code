# The Workers, as one platform

This repository holds eighteen Cloudflare Workers. On 2026-11-23,
fourteen answered at their addresses. Two of those are deployed from
GitHub Actions (the PM console and the pkarr door). The other twelve
were deployed by someone running `wrangler deploy` on a machine, and
nothing in the repository says which commit each is running.

[`workers.json`](workers.json) lists all eighteen: where each lives,
what it does, whether it is in service, how it gets there, and what
joining the platform means for it. The PM app's Platform screen (More,
Platform) is drawn from that file, read from the repository by the
edge, so the list changes with a merge and needs no machine.

## What "one platform" means

A Worker has joined when all of these are true:

1. **It is in the list**, under the name in its wrangler file.
2. **It is deployed from Actions.** A push to main that touches it
   uploads a version, puts it in service, asks it which version it is,
   and puts the previous one back if it does not answer as the new one.
   The PM console's release (`pm-stack/console/edge/release.mjs`) does
   this already, including standing a Worker up where there is none; it
   is the pattern to reuse, not to copy by hand.
3. **It says which version it is** at an address that needs no sign-in,
   so the deploy can check it and the Platform screen can show it.
4. **Its secrets are named in the repository and set by the deploy**,
   from the repository's own secret store, not typed on a machine.
5. **It is seen from the app**: its last deploy on the Platform screen,
   and anything it raises as one of the app's notifications.

After that, Workers that do one job together are merged where merging
removes a moving part: the decision events and the queue's alert into
the PM console's own state, the fleet's doors behind the console's
sign-in. Each merge is its own issue. What is left is tracked in
developing-today/code#13504.

## Keeping the list true

`workers.test.ts` holds the list to the repository: every tracked
wrangler file is in it under its own name and directory, nothing is in
it that is not in the repository, and a workflow it names exists, can
be started by hand and works in that Worker's directory. It runs with
the nightly pm-stack tests. A Worker added, moved or renamed without
the list being told fails there.

`state` is not something the repository can know. It was checked from
outside on the day in `checked` by asking each Worker's address: one
that is not deployed answers with Cloudflare's own "nothing here"
(error 1042), and anything else is the Worker's own answer. Checking it
from the edge itself is part of the work to come (a Worker cannot ask
another on the same account by its public address without a setting
the console does not have yet).

## Adding a Worker

Add it to `workers.json` in the same change that adds its wrangler
file: `name` as in that file, `path` its directory, `group` what it
belongs to, `state` and `deploy` as they are, `workflow` when Actions
deploys it, `address` when it is in service, and `next` in a sentence.
