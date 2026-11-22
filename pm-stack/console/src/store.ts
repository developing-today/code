// Where a small piece of state is kept between runs.
//
// The same classes (notifications, linked phones) run in two places: on
// the host, where state is a file, and at the edge, where it is the
// edge's own storage. They are given one of these and do not know which.

/** One JSON text, read whole and written whole. */
export interface Store {
  /** The text last written, or null when nothing has been. */
  read(): Promise<string | null>;
  write(text: string): Promise<void>;
}

/** Kept in memory: for tests, and for state that need not outlive the process. */
export function memoryStore(initial: string | null = null): Store & { text: () => string | null } {
  let text = initial;
  return {
    read: async () => text,
    write: async (next) => {
      text = next;
    },
    text: () => text,
  };
}
