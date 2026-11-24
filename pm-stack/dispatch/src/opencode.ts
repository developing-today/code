// Client for the laptop's standing OpenCode server (127.0.0.1:13420).
// Auth is HTTP Basic, user "opencode"; the password lives in the server's
// state file on the laptop and is read at runtime — never logged, never
// stored in this service's own state.
import { readFile } from "node:fs/promises";
import type { ModelRef } from "./validate.ts";
import { outboundTimeout } from "./fetchTimeout.ts";

// The OpenCode server caps `limit` at 200 and rejects a larger probe
// with HTTP 400 (#13149), so the cap itself is the only usable
// completeness probe: a response below 200 is complete; a response at
// 200 is unprovable and listSessions fails closed.
const SESSION_LIST_LIMITS = [200] as const;

export class OpenCodeError extends Error {
  status = 502;
  constructor(
    message: string,
    readonly httpStatus?: number,
  ) {
    super(message);
  }
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
    let res: Response;
    try {
      res = await fetch(`${this.baseUrl}${path}`, {
        method,
        headers: {
          Authorization: await this.authHeader(),
          "Content-Type": "application/json",
        },
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: outboundTimeout(),
      });
    } catch (e) {
      // A connection failure (server down, refused) is an upstream
      // failure, not a caller error: surface it as OpenCodeError so
      // routes answer the documented 502 instead of a raw 500.
      throw new OpenCodeError(`opencode ${method} ${path} unreachable: ${e instanceof Error ? e.message : String(e)}`);
    }
    const text = await res.text();
    if (!res.ok) {
      throw new OpenCodeError(
        `opencode ${method} ${path} failed (HTTP ${res.status}): ${text.slice(0, 200)}`,
        res.status,
      );
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

  /**
   * The bare `opencode-go` model rides the active OpenCode Console login.
   * Read that integration's active-first connection list so dispatch can
   * guard the same account and pin the created session to its explicit
   * account provider.
   */
  async activeIntegrationCredentialLabel(integrationId: string, directory: string): Promise<string> {
    const data = (await this.api("GET", `/api/integration?directory=${encodeURIComponent(directory)}`)) as unknown;
    if (!Array.isArray(data)) throw new OpenCodeError("opencode integration list returned no array");
    const integration = data.find(
      (item): item is { id: string; connections?: unknown } =>
        typeof item === "object" && item !== null && "id" in item && (item as { id?: unknown }).id === integrationId,
    );
    if (!integration || !Array.isArray(integration.connections)) {
      throw new OpenCodeError(`opencode integration ${integrationId} has no connection list`);
    }
    // OpenCode guarantees credentials are ordered active-first; environment
    // connections may follow but are not console accounts for this guard.
    const active = integration.connections.find(
      (connection): connection is { type: string; label?: unknown } =>
        typeof connection === "object" &&
        connection !== null &&
        "type" in connection &&
        (connection as { type?: unknown }).type === "credential",
    );
    if (!active || typeof active.label !== "string" || !active.label.trim()) {
      throw new OpenCodeError(`opencode integration ${integrationId} has no active credential label`);
    }
    return active.label.trim();
  }

  async sendPrompt(sessionId: string, text: string): Promise<void> {
    await this.api("POST", `/api/session/${encodeURIComponent(sessionId)}/prompt`, { text });
  }

  async deleteSession(sessionId: string): Promise<void> {
    await this.api("DELETE", `/api/session/${encodeURIComponent(sessionId)}`);
  }

  async listSessions(): Promise<unknown> {
    for (const limit of SESSION_LIST_LIMITS) {
      const data = await this.api("GET", `/api/session?limit=${limit}`);
      // OpenCode silently truncates the default session list at 50. A
      // response below its requested limit is the only completeness proof
      // this endpoint offers; if the largest supported probe is still full,
      // do not let cleanup infer that an omitted session is absent.
      if (!Array.isArray(data)) return data;
      if (data.length < limit) return data;
    }
    throw new OpenCodeError(
      `opencode session list reached the maximum completeness limit (${SESSION_LIST_LIMITS.at(-1)})`,
    );
  }

  /** Read one session's current state (issue #5555): the raw session
   * object from the OpenCode server, or null when the server says the
   * session does not exist (HTTP 404). Any other failure (server down,
   * auth, non-JSON) stays an OpenCodeError so callers can tell "gone"
   * apart from "unknown". */
  async getSession(sessionId: string): Promise<unknown> {
    try {
      return await this.api("GET", `/api/session/${encodeURIComponent(sessionId)}`);
    } catch (e) {
      // The 404 decision rides on the upstream status recorded by
      // api(), never on message text: a 500 whose body happens to
      // contain "(HTTP 404)" is a server failure, not a missing
      // session.
      if (e instanceof OpenCodeError && e.httpStatus === 404) return null;
      throw e;
    }
  }
}
