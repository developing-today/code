sections: protocol
kind: Changed
summary: cancelling an interruptible call now stops it on the ask path too — a notifications/cancelled naming the request routes to the daemon task cancel path (/v1/ask/{id}/abandon, the task store's Cancel); a transport that dies without a cancellation still leaves the task running and collectable (#1864 item 2)
