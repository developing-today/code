// The QueueRecordSource the stamping door consumes (#12585 left
// the seam defined-but-unwired for this door): queue fields for a
// stamp come from the queue's own records, served here from the
// queue store (#12589). A session is queue-bound when a task's
// progress carries its session_id — the claim/bind verbs write
// exactly that mark — and the record is the design §5 stamp set:
//
//   queue_id        "task-<id>" of the bound task (the fixture
//                   seed's spelling in #12585's tests)
//   scheduled_start the task's not_before (null once claimed —
//                   the claim clears it, so a stamp taken during
//                   work reads null, as the native record does)
//   picked_up_at    the claim record's claimed_at
//   work_started_at the session_bound event's time for this
//                   session, else the claim time
//   lane_start_issues — the queue record carries no lane-start set
//                   of its own; the stamping route prefers the
//                   session record's set, so this stays empty.
//
// Tenancy: the session must resolve through the session registry
// to the caller's own binding — another tenancy's session answers
// null, exactly like an unknown one (no existence oracle).

import type { QueueStore } from "./queue-store.ts";
import type { QueueRecord, QueueRecordSource } from "./stamping.ts";
import type { SessionRegistry } from "./sessions.ts";
import { isObj } from "./queue-validate.ts";

export class StoreQueueRecordSource implements QueueRecordSource {
  constructor(
    private readonly store: QueueStore,
    private readonly sessions: SessionRegistry,
  ) {}

  async recordForSession(bindingId: string, sessionId: string): Promise<QueueRecord | null> {
    const session = await this.sessions.get(sessionId);
    if (session === null || session.bindingId !== bindingId) return null;
    let best: {
      id: number;
      claimedAt: string | null;
      notBefore: string | null;
    } | null = null;
    for (const task of await this.store.allTasks()) {
      const progress = isObj(task.progress) ? task.progress : null;
      if (progress === null || progress["session_id"] !== sessionId) continue;
      if (
        best === null ||
        (task.claimed_at ?? "") > (best.claimedAt ?? "") ||
        ((task.claimed_at ?? "") === (best.claimedAt ?? "") && task.id > best.id)
      ) {
        best = { id: task.id, claimedAt: task.claimed_at, notBefore: task.not_before };
      }
    }
    if (best === null) return null;
    let workStartedAt: string | null = null;
    for (const ev of await this.store.eventsForTask(best.id)) {
      if (ev.kind !== "session_bound") continue;
      if (isObj(ev.detail) && ev.detail["session_id"] === sessionId) {
        workStartedAt = ev.created_at;
      }
    }
    return {
      queueId: `task-${best.id}`,
      scheduledStart: best.notBefore,
      pickedUpAt: best.claimedAt,
      workStartedAt: workStartedAt ?? best.claimedAt,
      laneStartIssues: [],
    };
  }
}
