export function createWatchdogHeartbeatRecorder(append: (atMs: number) => Promise<void>): {
  request(atMs: number): Promise<void>;
  scheduled(atMs: number): Promise<void>;
} {
  let lastSuccessfulAt: number | null = null;
  let requestWrite: Promise<void> | null = null;
  let writes = Promise.resolve();

  const record = (atMs: number): Promise<void> => {
    const write = writes.then(() => append(atMs));
    // Keep later appends usable after an individual write fails, while
    // returning the original rejection to the caller for its warning.
    writes = write.catch(() => {});
    return write.then(() => {
      lastSuccessfulAt = atMs;
    });
  };

  return {
    request(atMs) {
      if (lastSuccessfulAt !== null && atMs - lastSuccessfulAt < 5 * 60_000) {
        return requestWrite ?? writes;
      }
      if (requestWrite) return requestWrite;
      const write = record(atMs);
      requestWrite = write.finally(() => {
        requestWrite = null;
      });
      return requestWrite;
    },
    // Scheduled runs are liveness evidence in their own right. Do not let
    // the request-path write throttle suppress a successful timer pass.
    scheduled: record,
  };
}

export async function finishWatchdogGapSync<T>(options: {
  items: T;
  sync: Promise<void>;
  mode: "request" | "scheduled";
  recordHeartbeat: () => Promise<void>;
  onError: (error: unknown) => void;
}): Promise<T> {
  const completed = options.sync
    .then(() => options.recordHeartbeat())
    .catch((error: unknown) => {
      options.onError(error);
    });
  if (options.mode === "scheduled") await completed;
  else void completed;
  return options.items;
}
