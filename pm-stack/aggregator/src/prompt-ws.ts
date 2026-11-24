// sync-ws channel for prompt submissions (#10033 S3, design #9539 §3.5):
// a WebSocket on /ws/prompt/:id streams the prompt's state as it changes
// and closes when the prompt reaches a terminal state. The session logic
// lives here against a minimal socket interface so it is testable without
// a live server; server.ts adapts Bun's ServerWebSocket to it.
//
// The answer lands atomically through PATCH /api/prompt/:id, so a state
// snapshot is the stream unit: the channel sends the current snapshot on
// open and a fresh one whenever status/answer/error changes — the answer
// streams as it arrives, and the terminal snapshot is the last message
// before the close.
import { promptChannelMessage, type PromptsStore } from "./prompts.ts";

export interface PromptWsSocket {
  send(data: string): void;
  close(code?: number, reason?: string): void;
}

export interface PromptWsSession {
  start(): Promise<void>;
  /** Stop watching the store (the socket is gone or closing). */
  stop(): void;
}

export function createPromptWsSession(opts: {
  store: PromptsStore;
  promptId: string;
  socket: PromptWsSocket;
  /** Same configured wait as sync-block (design §3.5: 120s default):
   *  a channel whose prompt is still unanswered after it closes with a
   *  typed timeout message instead of hanging open forever. */
  timeoutMs: number;
  pollMs?: number;
}): PromptWsSession {
  const pollMs = Math.max(1, opts.pollMs ?? 100);
  let timer: ReturnType<typeof setInterval> | null = null;
  let stopped = false;
  let lastSent = "";

  function stop(): void {
    stopped = true;
    if (timer) {
      clearInterval(timer);
      timer = null;
    }
  }

  function send(data: string): boolean {
    if (data === lastSent) return false;
    lastSent = data;
    try {
      opts.socket.send(data);
      return true;
    } catch {
      stop();
      return false;
    }
  }

  function finish(code: number, reason: string): void {
    stop();
    try {
      opts.socket.close(code, reason);
    } catch {
      // The socket is already gone; nothing left to signal.
    }
  }

  async function start(): Promise<void> {
    const deadline = Date.now() + Math.max(0, opts.timeoutMs);
    const first = await opts.store.get(opts.promptId);
    if (!first) {
      send(JSON.stringify({ id: opts.promptId, status: "not-found", error: "prompt not found" }));
      finish(1008, "prompt not found");
      return;
    }
    send(JSON.stringify(promptChannelMessage(first)));
    if (first.status === "answered" || first.status === "failed") {
      finish(1000, "prompt complete");
      return;
    }
    timer = setInterval(() => {
      void (async () => {
        if (stopped) return;
        const record = await opts.store.get(opts.promptId);
        if (stopped) return;
        if (!record) {
          finish(1008, "prompt not found");
          return;
        }
        send(JSON.stringify(promptChannelMessage(record)));
        if (stopped) return;
        if (record.status === "answered" || record.status === "failed") {
          finish(1000, "prompt complete");
          return;
        }
        if (Date.now() >= deadline) {
          send(
            JSON.stringify({
              id: record.id,
              status: "timeout",
              error: `prompt not answered within ${opts.timeoutMs} ms`,
              timeoutMs: opts.timeoutMs,
            }),
          );
          finish(1000, "prompt answer timed out");
        }
      })();
    }, pollMs);
  }

  return { start, stop };
}
