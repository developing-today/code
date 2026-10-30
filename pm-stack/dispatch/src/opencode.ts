// Client for the laptop's standing OpenCode server (127.0.0.1:13420).
// Auth is HTTP Basic, user "opencode"; the password lives in the server's
// state file on the laptop and is read at runtime — never logged, never
// stored in this service's own state.
import { readFile } from "node:fs/promises";
import type { ModelRef } from "./validate.ts";

export class OpenCodeError extends Error {
  status = 502;
}

export class OpenCodeClient {
  constructor(
    private baseUrl: string,
    private authFile: string,
  ) {}

  private async authHeader(): Promise<string> {
    let raw: string;
    try {
      raw = await readFile(this.authFile, "utf8");
    } catch {
      throw new OpenCodeError(`opencode auth file unreadable: ${this.authFile}`);
    }
    const data = JSON.parse(raw) as { password?: string };
    if (!data.password) throw new OpenCodeError("opencode auth file carries no password");
    return `Basic ${Buffer.from(`opencode:${data.password}`).toString("base64")}`;
  }

  private async api(method: string, path: string, body?: unknown): Promise<unknown> {
    const res = await fetch(`${this.baseUrl}${path}`, {
      method,
      headers: {
        Authorization: await this.authHeader(),
        "Content-Type": "application/json",
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await res.text();
    if (!res.ok) {
      throw new OpenCodeError(`opencode ${method} ${path} failed (HTTP ${res.status}): ${text.slice(0, 200)}`);
    }
    if (!text.trim()) return null;
    const parsed: unknown = JSON.parse(text);
    // The v2 API wraps responses in a {data: …} envelope.
    if (typeof parsed === "object" && parsed !== null && "data" in parsed) {
      return (parsed as { data: unknown }).data;
    }
    return parsed;
  }

  async createSession(opts: {
    directory: string;
    title: string;
    model: ModelRef | null;
  }): Promise<{ id: string }> {
    const body: Record<string, unknown> = {
      title: opts.title,
      location: { directory: opts.directory },
    };
    if (opts.model) body.model = { id: opts.model.id, providerID: opts.model.providerID };
    const data = (await this.api("POST", "/api/session", body)) as { id?: string } | null;
    if (!data || typeof data.id !== "string" || !data.id) {
      throw new OpenCodeError("opencode session create returned no session id");
    }
    return { id: data.id };
  }

  async sendPrompt(sessionId: string, text: string): Promise<void> {
    await this.api("POST", `/api/session/${encodeURIComponent(sessionId)}/prompt`, { text });
  }

  async listSessions(): Promise<unknown> {
    return this.api("GET", "/api/session");
  }
}
