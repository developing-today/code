// In-memory KVNamespace stub for tests: just enough of the Workers KV
// surface the OAuth module uses (get with "json", put with
// expirationTtl, delete), with lazy TTL expiry like the real thing.
export class MemoryKV {
  private store = new Map<string, { value: string; expiresAt?: number }>();

  async get(key: string, type?: string): Promise<unknown> {
    const entry = this.store.get(key);
    if (!entry) return null;
    if (entry.expiresAt !== undefined && entry.expiresAt <= Date.now()) {
      this.store.delete(key);
      return null;
    }
    return type === "json" ? JSON.parse(entry.value) : entry.value;
  }

  async put(key: string, value: string, options?: { expirationTtl?: number }): Promise<void> {
    this.store.set(key, {
      value,
      expiresAt: options?.expirationTtl ? Date.now() + options.expirationTtl * 1000 : undefined,
    });
  }

  async delete(key: string): Promise<void> {
    this.store.delete(key);
  }

  // Test-only inspection helpers.
  keys(): string[] {
    return [...this.store.keys()];
  }
}

export function memoryKvBinding(): KVNamespace {
  return new MemoryKV() as unknown as KVNamespace;
}
