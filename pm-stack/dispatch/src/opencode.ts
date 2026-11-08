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
    // The parse and the shape are part of the boundary (#7085): a
    // truncated/mid-write state file, a non-object document, or a
    // non-string password must surface as an OpenCodeError naming the
    // problem - never a raw Type/SyntaxError, and never a credential
    // silently coerced from a number or object.
    let parsedAuth: unknown;
    try {
      parsedAuth = JSON.parse(raw);
    } catch {
      throw new OpenCodeError(`opencode auth file is not valid JSON: ${this.authFile}`);
    }
    if (typeof parsedAuth !== "object" || parsedAuth === null || Array.isArray(parsedAuth)) {
      throw new OpenCodeError(`opencode auth file does not hold a JSON object: ${this.authFile}`);
    }
    const data = parsedAuth as { password?: unknown };
    if (typeof data.password !== "string" || !data.password) {
      throw new OpenCodeError("opencode auth file carries no password");
    }
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
    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch {
      throw new OpenCodeError(`opencode ${method} ${path} returned a non-JSON response (HTTP ${res.status})`);
    }
    // The v2 API wraps responses in a {data: …} envelope.
    if (typeof parsed === "object" && parsed !== null && "data" in parsed) {
      return (parsed as { data: unknown }).data;
    }
    return parsed;
  }

  async createSession(opts: { directory: string; title: string; model: ModelRef | null }): Promise<{ id: string }> {
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

  async deleteSession(sessionId: string): Promise<void> {
    await this.api("DELETE", `/api/session/${encodeURIComponent(sessionId)}`);
  }

  async listSessions(): Promise<unknown> {
    return this.api("GET", "/api/session");
  }
}
