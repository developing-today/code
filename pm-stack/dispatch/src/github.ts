// GitHub issue creation for dispatch tracking issues.
//
// Primary route: the pm-stack GitHub App (installation token minted from
// the app id + private key rendered into the service env from
// ~/secrets/pm-stack.sops.yaml — the same material connect-repo.sh uses).
// The installations cover developing-today/code, developing-today/opencode,
// developing-today/openchamber and dezren39/nix; callers skip repos with no
// installation (developing-today/zettel, dezren39/dezren39) before reaching
// here.
//
// Fallback: if the app route is unusable (no app credentials in env, or
// the installation/token call fails), use the laptop's authenticated
// `gh` CLI. Either way failures surface as GitHubError with both routes'
// errors summarized — issue creation is never silently skipped.
import { createSign } from "node:crypto";

const GH_API = "https://api.github.com";

export class GitHubError extends Error {
  status = 502;
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

export class GitHubClient {
  private installationIdCache = new Map<string, number>();
  private tokenCache = new Map<string, { token: string; expiresAt: number }>();

  constructor(
    private appId: string | null,
    private appPrivateKeyPem: string | null,
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
      const data = (await res.json()) as { id: number };
      installationId = data.id;
      this.installationIdCache.set(repo, installationId);
    }
    const res = await this.appApi("POST", `/app/installations/${installationId}/access_tokens`, jwt);
    if (!res.ok) throw new GitHubError(`github app token mint failed (HTTP ${res.status})`);
    const data = (await res.json()) as { token: string; expires_at: string };
    this.tokenCache.set(repo, { token: data.token, expiresAt: Date.parse(data.expires_at) });
    return data.token;
  }

  // --- gh CLI fallback -------------------------------------------------------

  private async gh(args: string[], input?: string): Promise<string> {
    const proc = Bun.spawn(["gh", ...args], {
      stdin: input !== undefined ? new Blob([input]) : "ignore",
      stdout: "pipe",
      stderr: "pipe",
    });
    const [out, err, code] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
      proc.exited,
    ]);
    if (code !== 0) throw new GitHubError(`gh ${args[0]} ${args[1] ?? ""} failed: ${err.trim().slice(0, 200)}`);
    return out;
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
      // gh fallback for label creation is best-effort: `gh issue create
      // --label` fails on a missing label, so create it there too.
      try {
        await this.gh(["label", "create", label, "--repo", repo, "--color", def.color, "--description", def.description, "--force"]);
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
        const res = await this.appApi("POST", `/repos/${repo}/issues`, token, { title, body, labels });
        if (!res.ok) {
          throw new GitHubError(`github issue create failed (HTTP ${res.status}): ${(await res.text()).slice(0, 200)}`);
        }
        const data = (await res.json()) as { number: number; html_url: string };
        return { number: data.number, url: data.html_url };
      } catch (e) {
        // Fall through to gh with the app error preserved in the message
        // if gh fails too.
        try {
          return await this.createIssueViaGh(repo, title, body, labels);
        } catch (ghErr) {
          throw new GitHubError(
            `issue create failed via app (${e instanceof Error ? e.message : String(e)}) and via gh (${ghErr instanceof Error ? ghErr.message : String(ghErr)})`,
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
    const args = ["issue", "create", "--repo", repo, "--title", title, "--body", body];
    for (const l of labels) args.push("--label", l);
    const out = await this.gh(args);
    const url = out.trim().split("\n").pop()!.trim();
    const m = /\/issues\/(\d+)$/.exec(url);
    if (!m) throw new GitHubError(`gh issue create returned an unparseable URL: ${url.slice(0, 120)}`);
    return { number: Number(m[1]), url };
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
        /* fall through to gh */
      }
    }
    await this.gh(["issue", "edit", String(issueNumber), "--repo", repo, ...labels.flatMap((l) => ["--add-label", l])]);
  }

  async comment(repo: string, issueNumber: number, body: string): Promise<void> {
    if (this.appConfigured) {
      try {
        const token = await this.installationToken(repo);
        const res = await this.appApi("POST", `/repos/${repo}/issues/${issueNumber}/comments`, token, { body });
        if (!res.ok) throw new GitHubError(`issue comment failed (HTTP ${res.status})`);
        return;
      } catch {
        /* fall through to gh */
      }
    }
    await this.gh(["issue", "comment", String(issueNumber), "--repo", repo, "--body", body]);
  }
}
