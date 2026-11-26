// GitHub issue creation for dispatch tracking issues.
//
// Primary route: the pm-stack GitHub App (installation token minted from
// the app id + private key rendered into the service env from
// ~/secrets/pm-stack.sops.yaml — the same material connect-repo.sh uses).
// The installations cover developing-today/code and
// dezren39/darwin-nix (APP_INSTALLED_REPOS in config.ts is the list
// of record); callers skip repos with no installation
// (developing-today-forks/opencode, developing-today-forks/openchamber,
// developing-today/zettel, dezren39/dezren39) before reaching here.
//
// Fallback: if the app route is unusable (no app credentials in env, or
// the installation/token call fails), use the fleet gh sync verbs
// (code#10973, step 2.2 of #9016) on the laptop's packaged fleet
// binary, over the fleet token rotation. Either way failures surface
// as GitHubError with both routes' errors summarized — issue creation
// is never silently skipped.
import { createSign } from "node:crypto";
import { join } from "node:path";
import { OUTBOUND_FETCH_TIMEOUT_MS, isTimeoutAbort, outboundTimeout } from "./fetchTimeout.ts";
import { requireHome } from "./home.ts";

/** The packaged fleet binary the fallback's sync verbs run through:
 * PM_DISPATCH_FLEET_BIN overrides; the default is the packaged
 * location the opencode claimant also uses (~/.local/bin/fleet). Resolved
 * at spawn time, so a missing HOME fails only the fallback that needs it
 * (#8220). */
const fleetBin = (): string =>
  process.env.PM_DISPATCH_FLEET_BIN ?? join(requireHome("PM_DISPATCH_FLEET_BIN"), ".local", "bin", "fleet");

const GH_API = "https://api.github.com";

/** Bound for one gh CLI fallback call (#6992): the same 15 s discipline
 * the fetch routes carry (#10825), enforced on the subprocess. */
export const GH_SUBPROCESS_TIMEOUT_MS = OUTBOUND_FETCH_TIMEOUT_MS;

export class GitHubError extends Error {
  status = 502;
}

/**
 * A 2xx response body whose shape does not match the endpoint's contract
 * (#7101). The dispatch sibling of the projector fix in #6954: a
 * wrong-shape body must surface as a GitHubError naming the endpoint and
 * the malformed field — never a raw TypeError/SyntaxError, never a
 * coerced or undefined value flowing downstream.
 */
export class MalformedBodyError extends GitHubError {}

/** The app create-issue call answered 2xx with a malformed body: the
 * issue may already exist server-side, so the gh fallback create must
 * NOT run (it would file a duplicate tracking issue). */
class MalformedCreateBodyError extends MalformedBodyError {}

/** The app create-issue POST timed out after it was sent: GitHub may have
 * created the issue and only the response was slow, so, as with a
 * malformed 2xx body, the gh fallback create must NOT run (#10849). */
export class AmbiguousCreateError extends GitHubError {}

function describeValue(v: unknown): string {
  if (v === null) return "null";
  if (Array.isArray(v)) return "array";
  return typeof v;
}

async function jsonBody(res: Response, endpoint: string): Promise<unknown> {
  try {
    return await res.json();
  } catch {
    throw new MalformedBodyError(`github ${endpoint} returned a non-JSON body`);
  }
}

function bodyObject(body: unknown, endpoint: string): Record<string, unknown> {
  if (body === null || typeof body !== "object" || Array.isArray(body)) {
    throw new MalformedBodyError(
      `github ${endpoint} returned a malformed body: expected a JSON object, got ${describeValue(body)}`,
    );
  }
  return body as Record<string, unknown>;
}

function reqPositiveInt(obj: Record<string, unknown>, field: string, endpoint: string): number {
  const v = obj[field];
  if (typeof v !== "number" || !Number.isInteger(v) || v <= 0) {
    throw new MalformedBodyError(
      `github ${endpoint} returned a malformed body: field '${field}' must be a positive integer, got ${describeValue(v)}`,
    );
  }
  return v;
}

function reqInt(obj: Record<string, unknown>, field: string, endpoint: string): number {
  const v = obj[field];
  if (typeof v !== "number" || !Number.isInteger(v)) {
    throw new MalformedBodyError(
      `github ${endpoint} returned a malformed body: field '${field}' must be an integer, got ${describeValue(v)}`,
    );
  }
  return v;
}

function reqString(obj: Record<string, unknown>, field: string, endpoint: string, nonEmpty = false): string {
  const v = obj[field];
  if (typeof v !== "string" || (nonEmpty && v === "")) {
    throw new MalformedBodyError(
      `github ${endpoint} returned a malformed body: field '${field}' must be a ${nonEmpty ? "non-empty " : ""}string, got ${describeValue(v)}`,
    );
  }
  return v;
}

/** Label -> color/description when dispatch has to create it. Colors match
 * the fleet taxonomy as used in developing-today/code. */
export const LABEL_DEFS: Record<string, { color: string; description: string }> = {
  "source/dispatch": { color: "2da44e", description: "Filed by pm-dispatch" },
  "queue/queued": { color: "7057ff", description: "Has a queued queue task" },
  "queue/claimed": { color: "5319e7", description: "Queue task claimed by a worker" },
  "work/assigned": { color: "1f6feb", description: "Held by a queue worker" },
};

function b64url(data: Buffer | string): string {
  return Buffer.from(data).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/**
 * True only when a gh CLI error reports an actual HTTP 404. Matches the
 * status text ("HTTP 404" / "(HTTP 404)"), never the bare digits: the gh
 * error prefix echoes the request path, so an issue literally numbered
 * 404 (".../issues/404") must not turn an auth/rate-limit/transient
 * failure into a definitive not-found (Codex P2 on PR #3149).
 */
export function isNotFoundError(message: string): boolean {
  return /HTTP 404\b/.test(message);
}

export interface IssueInfo {
  number: number;
  state: string;
  isPullRequest: boolean;
  url: string;
}

export class GitHubClient {
  private installationIdCache = new Map<string, number>();
  private tokenCache = new Map<string, { token: string; expiresAt: number }>();

  constructor(
    private appId: string | null,
    private appPrivateKeyPem: string | null,
    /** gh CLI fallback binary; a constructor seam so tests can point the
     * fallback at a fake `gh` (Bun.spawn does not re-read a mutated PATH). */
    private ghBinary = "gh",
  ) {}

  get appConfigured(): boolean {
    return Boolean(this.appId && this.appPrivateKeyPem);
  }

  private appJwt(): string {
    if (!this.appConfigured) throw new GitHubError("github app credentials not configured");
    const now = Math.floor(Date.now() / 1000);
    const header = b64url(JSON.stringify({ alg: "RS256", typ: "JWT" }));
    const payload = b64url(JSON.stringify({ iat: now - 60, exp: now + 540, iss: this.appId }));
    const signer = createSign("RSA-SHA256");
    signer.update(`${header}.${payload}`);
    const sig = b64url(signer.sign(this.appPrivateKeyPem!));
    return `${header}.${payload}.${sig}`;
  }

  private async appApi(method: string, path: string, token: string, body?: unknown): Promise<Response> {
    return fetch(`${GH_API}${path}`, {
      method,
      headers: {
        Authorization: `Bearer ${token}`,
        Accept: "application/vnd.github+json",
        "Content-Type": "application/json",
        "User-Agent": "pm-dispatch",
      },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: outboundTimeout(),
    });
  }

  private async installationToken(repo: string): Promise<string> {
    const cached = this.tokenCache.get(repo);
    if (cached && cached.expiresAt > Date.now() + 60_000) return cached.token;

    const jwt = this.appJwt();
    let installationId = this.installationIdCache.get(repo);
    if (installationId === undefined) {
      const res = await this.appApi("GET", `/repos/${repo}/installation`, jwt);
      if (!res.ok) {
        throw new GitHubError(
          `github app installation lookup for ${repo} failed (HTTP ${res.status}) — app not installed there?`,
        );
      }
      const endpoint = `GET /repos/${repo}/installation`;
      const data = bodyObject(await jsonBody(res, endpoint), endpoint);
      installationId = reqPositiveInt(data, "id", endpoint);
      this.installationIdCache.set(repo, installationId);
    }
    const res = await this.appApi("POST", `/app/installations/${installationId}/access_tokens`, jwt);
    if (!res.ok) throw new GitHubError(`github app token mint failed (HTTP ${res.status})`);
    const endpoint = `POST /app/installations/${installationId}/access_tokens`;
    const data = bodyObject(await jsonBody(res, endpoint), endpoint);
    const token = reqString(data, "token", endpoint, true);
    const expiresAtRaw = reqString(data, "expires_at", endpoint, true);
    const expiresAt = Date.parse(expiresAtRaw);
    if (Number.isNaN(expiresAt)) {
      throw new MalformedBodyError(
        `github ${endpoint} returned a malformed body: field 'expires_at' is not a parseable timestamp`,
      );
    }
    this.tokenCache.set(repo, { token, expiresAt });
    return token;
  }

  // --- fleet gh sync-verb fallback -------------------------------------------
  //
  // Every fallback call runs one typed sync verb (code#10973) instead
  // of composing raw gh arguments here: the verb validates its whole
  // command line and rewrites to the canonical gh call over the fleet
  // token rotation, and its stdout is the underlying call's stdout,
  // byte-exact, so the parsers below keep their shapes.

  private async fleetGh(args: string[], timeoutMs = GH_SUBPROCESS_TIMEOUT_MS): Promise<string> {
    let proc;
    try {
      proc = Bun.spawn([fleetBin(), "gh", ...args], {
        stdin: "ignore",
        stdout: "pipe",
        stderr: "pipe",
      });
    } catch (e) {
      throw new GitHubError(`fleet gh ${args[0]} failed to start: ${e instanceof Error ? e.message : String(e)}`);
    }
    // The fetch routes are bounded (#10825) and so is the direct-gh
    // fallback below: a subprocess parked on API rate-limit retries
    // must never hold the launch handler open indefinitely (#6992).
    // The fleet path carries the same bound.
    let timer: ReturnType<typeof setTimeout> | undefined;
    const deadline = new Promise<never>((_, reject) => {
      timer = setTimeout(() => {
        proc.kill();
        reject(new GitHubError(`fleet gh ${args[0]} timed out after ${timeoutMs} ms — killed`));
      }, timeoutMs);
    });
    try {
      const [out, err, code] = await Promise.race([
        Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text(), proc.exited]),
        deadline,
      ]);
      if (code !== 0) throw new GitHubError(`fleet gh ${args[0]} failed: ${err.trim().slice(0, 200)}`);
      return out;
    } finally {
      if (timer !== undefined) clearTimeout(timer);
    }
  }

  private async gh(args: string[], input?: string, timeoutMs = GH_SUBPROCESS_TIMEOUT_MS): Promise<string> {
    const proc = Bun.spawn([this.ghBinary, ...args], {
      stdin: input !== undefined ? new Blob([input]) : "ignore",
      stdout: "pipe",
      stderr: "pipe",
    });
    // The fetch routes are bounded (#10825) but this subprocess was not:
    // a gh call parked on API rate-limit retries held the launch handler
    // open indefinitely (#6992). Kill the subprocess at the same bound
    // and surface the timeout as a GitHubError, like any other gh failure.
    let timer: ReturnType<typeof setTimeout> | undefined;
    const deadline = new Promise<never>((_, reject) => {
      timer = setTimeout(() => {
        proc.kill();
        reject(new GitHubError(`gh ${args[0]} ${args[1] ?? ""} timed out after ${timeoutMs} ms — killed`));
      }, timeoutMs);
    });
    try {
      const [out, err, code] = await Promise.race([
        Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text(), proc.exited]),
        deadline,
      ]);
      if (code !== 0) throw new GitHubError(`gh ${args[0]} ${args[1] ?? ""} failed: ${err.trim().slice(0, 200)}`);
      return out;
    } finally {
      if (timer !== undefined) clearTimeout(timer);
    }
  }

  // --- public surface --------------------------------------------------------

  async ensureLabel(repo: string, label: string): Promise<void> {
    const def = LABEL_DEFS[label];
    if (!def) return;
    try {
      const token = await this.installationToken(repo);
      const res = await this.appApi("GET", `/repos/${repo}/labels/${encodeURIComponent(label)}`, token);
      if (res.status === 404) {
        await this.appApi("POST", `/repos/${repo}/labels`, token, {
          name: label,
          color: def.color,
          description: def.description,
        });
      }
    } catch {
      // The verb fallback for label creation is best-effort: issue
      // create with a missing label fails, so ensure it there too.
      try {
        await this.fleetGh([
          "sync-label-ensure",
          "--repo",
          repo,
          "--label",
          label,
          "--color",
          def.color,
          "--description",
          def.description,
        ]);
      } catch {
        /* label may already exist — the create call below will say so */
      }
    }
  }

  async createIssue(
    repo: string,
    title: string,
    body: string,
    labels: string[],
  ): Promise<{ number: number; url: string }> {
    for (const label of labels) await this.ensureLabel(repo, label);
    if (this.appConfigured) {
      try {
        const token = await this.installationToken(repo);
        let res: Response;
        try {
          res = await this.appApi("POST", `/repos/${repo}/issues`, token, { title, body, labels });
        } catch (postErr) {
          if (isTimeoutAbort(postErr)) {
            throw new AmbiguousCreateError(
              `github issue create timed out after the request was sent; the issue may exist (${repo}: ${title.slice(0, 80)})`,
            );
          }
          throw postErr;
        }
        if (!res.ok) {
          throw new GitHubError(`github issue create failed (HTTP ${res.status}): ${(await res.text()).slice(0, 200)}`);
        }
        const endpoint = `POST /repos/${repo}/issues`;
        let data: Record<string, unknown>;
        try {
          data = bodyObject(await jsonBody(res, endpoint), endpoint);
        } catch (shapeErr) {
          throw new MalformedCreateBodyError(shapeErr instanceof Error ? shapeErr.message : String(shapeErr));
        }
        let number: number;
        let url: string;
        try {
          number = reqInt(data, "number", endpoint);
          url = reqString(data, "html_url", endpoint);
        } catch (shapeErr) {
          throw new MalformedCreateBodyError(shapeErr instanceof Error ? shapeErr.message : String(shapeErr));
        }
        return { number, url };
      } catch (e) {
        // A malformed 2xx create body is NOT a fallback case: the app
        // POST may already have created the issue server-side, so a gh
        // create would file a duplicate tracking issue (#7101).
        if (e instanceof MalformedCreateBodyError || e instanceof AmbiguousCreateError) throw e;
        // Fall through to the fleet gh verb with the app error
        // preserved in the message if the verb fails too.
        try {
          return await this.createIssueViaGh(repo, title, body, labels);
        } catch (ghErr) {
          throw new GitHubError(
            `issue create failed via app (${e instanceof Error ? e.message : String(e)}) and via fleet gh (${ghErr instanceof Error ? ghErr.message : String(ghErr)})`,
          );
        }
      }
    }
    return this.createIssueViaGh(repo, title, body, labels);
  }

  private async createIssueViaGh(
    repo: string,
    title: string,
    body: string,
    labels: string[],
  ): Promise<{ number: number; url: string }> {
    const args = ["sync-issue-create", "--repo", repo, "--title", title, "--body", body];
    for (const l of labels) args.push("--label", l);
    const out = await this.fleetGh(args);
    const url = out.trim().split("\n").pop()!.trim();
    const m = /\/issues\/(\d+)$/.exec(url);
    if (!m) throw new GitHubError(`fleet gh sync-issue-create returned an unparseable URL: ${url.slice(0, 120)}`);
    return { number: Number(m[1]), url };
  }

  /**
   * Look up an issue by number (issue #3007). Returns null on a
   * definitive 404; throws GitHubError for any other failure so callers
   * can distinguish an invalid target from a lookup failure. The GitHub
   * issues API also returns pull requests — `isPullRequest` exposes the
   * `pull_request` marker so callers can reject PR numbers.
   */
  async getIssue(repo: string, issueNumber: number): Promise<IssueInfo | null> {
    let appError: unknown = null;
    if (this.appConfigured) {
      try {
        const token = await this.installationToken(repo);
        const res = await this.appApi("GET", `/repos/${repo}/issues/${issueNumber}`, token);
        if (res.status === 404) return null;
        if (!res.ok) throw new GitHubError(`github issue lookup failed (HTTP ${res.status})`);
        const endpoint = `GET /repos/${repo}/issues/${issueNumber}`;
        const data = bodyObject(await jsonBody(res, endpoint), endpoint);
        return {
          number: reqInt(data, "number", endpoint),
          state: reqString(data, "state", endpoint),
          isPullRequest: Boolean(data.pull_request),
          url: reqString(data, "html_url", endpoint),
        };
      } catch (e) {
        appError = e;
        /* fall through to the fleet gh verb, mirroring createIssue */
      }
    }
    try {
      const out = await this.fleetGh(["sync-issue-get", "--repo", repo, String(issueNumber)]);
      const endpoint = `fleet gh sync-issue-get repos/${repo}/issues/${issueNumber}`;
      let parsed: unknown;
      try {
        parsed = JSON.parse(out);
      } catch {
        throw new MalformedBodyError(`github ${endpoint} returned a non-JSON body`);
      }
      const data = bodyObject(parsed, endpoint);
      return {
        number: reqInt(data, "number", endpoint),
        state: reqString(data, "state", endpoint),
        isPullRequest: Boolean(data.pull_request),
        url: reqString(data, "html_url", endpoint),
      };
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (isNotFoundError(msg)) return null;
      if (appError) {
        throw new GitHubError(
          `issue lookup failed via app (${appError instanceof Error ? appError.message : String(appError)}) and via fleet gh (${msg})`,
        );
      }
      throw e;
    }
  }

  async addLabels(repo: string, issueNumber: number, labels: string[]): Promise<void> {
    for (const label of labels) await this.ensureLabel(repo, label);
    if (this.appConfigured) {
      try {
        const token = await this.installationToken(repo);
        const res = await this.appApi("POST", `/repos/${repo}/issues/${issueNumber}/labels`, token, { labels });
        if (!res.ok) throw new GitHubError(`add labels failed (HTTP ${res.status})`);
        return;
      } catch {
        /* fall through to the fleet gh verb */
      }
    }
    await this.fleetGh([
      "sync-labels-add",
      "--repo",
      repo,
      String(issueNumber),
      ...labels.flatMap((l) => ["--label", l]),
    ]);
  }

  /**
   * Close an issue with a state reason (#4917). Used to compensate a
   * dispatch-created tracking issue when the launch fails before the
   * dispatch becomes real, so the issue does not linger open as an
   * orphan. Only ever called on issues this service created itself —
   * never on attach targets a caller supplied.
   */
  async closeIssue(
    repo: string,
    issueNumber: number,
    stateReason: "not_planned" | "completed" = "not_planned",
  ): Promise<void> {
    if (this.appConfigured) {
      try {
        const token = await this.installationToken(repo);
        const res = await this.appApi("PATCH", `/repos/${repo}/issues/${issueNumber}`, token, {
          state: "closed",
          state_reason: stateReason,
        });
        if (!res.ok) throw new GitHubError(`issue close failed (HTTP ${res.status})`);
        return;
      } catch {
        /* fall through to the fleet gh verb */
      }
    }
    // Through the verb the close speaks the fleet closure taxonomy
    // (#9221): not_planned closes as `cancelled`, completed as
    // `implemented`, and the close carries the closure reason
    // comment + record. GitHub's own state_reason granularity is
    // not preserved; every live caller closes with not_planned, to
    // compensate a dispatch-created tracking issue whose launch
    // failed (launch_failure.ts).
    await this.fleetGh([
      "sync-issue-close",
      "--repo",
      repo,
      String(issueNumber),
      "--reason",
      stateReason === "not_planned" ? "cancelled" : "implemented",
    ]);
  }

  async comment(repo: string, issueNumber: number, body: string): Promise<void> {
    if (this.appConfigured) {
      try {
        const token = await this.installationToken(repo);
        const res = await this.appApi("POST", `/repos/${repo}/issues/${issueNumber}/comments`, token, { body });
        if (!res.ok) throw new GitHubError(`issue comment failed (HTTP ${res.status})`);
        return;
      } catch {
        /* fall through to the fleet gh verb */
      }
    }
    await this.fleetGh(["sync-comment", "--repo", repo, "issue", String(issueNumber), "--body", body]);
  }
}
