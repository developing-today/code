// One instance per limited key, so the count is exact; the Rate Limiting binding is per machine and too loose for a recipient cap.
export class Limiter {
  constructor(state) {
    this.storage = state.storage;
  }

  async fetch(request) {
    const { limit, windowMs } = await request.json();
    const now = Date.now();
    const stored = await this.storage.get(["count", "windowStart"]);
    let count = stored.get("count") ?? 0;
    let windowStart = stored.get("windowStart") ?? now;
    if (now - windowStart >= windowMs) {
      count = 0;
      windowStart = now;
    }
    if (count >= limit) {
      return Response.json({ success: false });
    }
    await this.storage.put({ count: count + 1, windowStart });
    return Response.json({ success: true });
  }
}
