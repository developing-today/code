# Fleet alerts channel (do not merge)

This file exists only on the `fleet-alerts` branch, whose draft pull
request is a standing channel for fleet alerts. Monitors post one
comment on that PR when something goes down and one when it recovers,
and the chat session watching the PR relays each comment to Drew.

Today the only poster is the queue transport alert Worker
(`pkgs/queue-transport-alert/worker`, #6369), configured with the PR
number in its `ALERTS_PR` var.

Never merge or close this PR; it carries no change.
