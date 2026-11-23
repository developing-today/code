// The GitHub write seam + the hosted rotation point (#12585;
// design §3.1, §2).
//
// Hosted, every GitHub write goes through a verb that can stamp —
// there is no raw passthrough — and the write itself is performed by
// the service through the fleet App's installation-token rotation,
// held as Worker secrets. That is #9016's "one rotation point"
// property, relocated server-side: callers never see a GitHub token.
//
// The rotation mirrors the native fleet gh semantics (#8887, design
// §1): installation tokens are tried in order, and a rate-limit or
// permission refusal fails over to the next installation; any other
// failure surfaces. Tokens are minted per installation from an App
// JWT (RS256, WebCrypto) exchanged at GitHub's access_tokens
// endpoint, and cached until shortly before expiry. The App's
// private key is configuration referenced by name only (see
// config.ts: GITHUB_APP_*), parsed to bytes there, never logged.
//
// SEAM: the writer is an interface with a fixture implementation —
// the TenancyDirectory / SessionRegistry idiom. The Worker wires the
// rotation writer when App configuration is present and the fixture
// otherwise (local/dev semantics; this package is not deployed — see
// wrangler.toml). Tests inject either, plus a fake fetch for the
// rotation writer, so no test touches the network.

import type { GitHubAppConfig } from "./config.ts";
import { FailClosed } from "./errors.ts";

export interface GitHubCreateInput {
  /** "owner/name" — a claim the CLI assembles (design §3.2). */
  repo: string;
  kind: "issue" | "pr";
  title: string;
  /** The full body, Metadata block already stamped in. */
  body: string;
  /** PR only: the head branch claim (design §3.2). */
  head?: string;
  /** PR only: the base branch claim. */
  base?: string;
}

export interface GitHubCreatedItem {
  number: number;
  url: string;
}

export interface GitHubWriter {
  createItem(input: GitHubCreateInput): Promise<GitHubCreatedItem>;
}

/** One GitHub item as the query surface reads it (#12587): identity
 * plus the current body text, from which the Metadata block is
 * parsed. */
export interface GitHubItem {
  repo: string;
  kind: "issue" | "pr";
  number: number;
  url: string;
  body: string;
}

/** The GitHub read seam for the query surface (#12587; #9060 §13's
 * fallback + §15's body-vs-DB divergence check). The query door
 * reads bodies through this interface only: the current body is what
 * a hand-edit changes, so divergence can only be judged against a
 * fresh read, never against the writer's memory of the create. The
 * App rotation class below implements it over the same installation
 * tokens as the writes; the fixture writer implements it over its
 * recorded items (mutating a recorded body is how a test simulates
 * a hand-edit). */
export interface GitHubItemSource {
  /** The item's current state, or null when GitHub has no such
   * item (or it is not visible to the rotation). */
  getItem(repo: string, kind: "issue" | "pr", number: number): Promise<GitHubItem | null>;
  /** Every item in the repo (issues and PRs, any state), for the
   * not-yet-ingested fallback scan. Bounded: the App implementation
   * follows at most MAX_LIST_PAGES pages of 100. */
  listItems(repo: string): Promise<GitHubItem[]>;
}

/** Fixture writer: records every created item in memory (bodies read
 * back exactly as written, which is how the endpoint's acceptance is
 * checked locally) and numbers items per repo, issues and PRs in one
 * sequence as GitHub does. Also the fixture GitHubItemSource: reads
 * answer from the same recorded items, so a test that mutates a
 * recorded body is simulating a post-create hand-edit on GitHub. */
export class FixtureGitHubWriter implements GitHubWriter, GitHubItemSource {
  readonly items: (GitHubCreateInput & GitHubCreatedItem)[] = [];
  private readonly counters = new Map<string, number>();

  async createItem(input: GitHubCreateInput): Promise<GitHubCreatedItem> {
    const number = (this.counters.get(input.repo) ?? 0) + 1;
    this.counters.set(input.repo, number);
    const url = `https://github.com/${input.repo}/${input.kind === "pr" ? "pull" : "issues"}/${number}`;
    const item = { ...input, number, url };
    this.items.push(item);
    return { number, url };
  }

  async getItem(repo: string, kind: "issue" | "pr", number: number): Promise<GitHubItem | null> {
    const item = this.items.find((i) => i.repo === repo && i.kind === kind && i.number === number);
    if (!item) return null;
    return { repo: item.repo, kind: item.kind, number: item.number, url: item.url, body: item.body };
  }

  async listItems(repo: string): Promise<GitHubItem[]> {
    return this.items
      .filter((i) => i.repo === repo)
      .map((i) => ({ repo: i.repo, kind: i.kind, number: i.number, url: i.url, body: i.body }));
  }
}

// --- The App installation-token rotation (production writer).

export type FetchFn = (url: string, init: RequestInit) => Promise<Response>;

const GITHUB_API = "https://api.github.com";
/** App JWTs may live at most 10 minutes; mint at 9, backdate iat a
 * minute for clock drift (GitHub's documented practice). */
const APP_JWT_TTL_SECONDS = 540;
const APP_JWT_BACKDATE_SECONDS = 60;
/** Refresh an installation token this long before its expiry. */
const TOKEN_REFRESH_MARGIN_MS = 60_000;

function b64urlEncode(data: string | Uint8Array): string {
  const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

interface CachedToken {
  token: string;
  expiresAtMs: number;
}

/**
 * The hosted rotation point: creates issues/PRs with fleet App
 * installation tokens, failing over across the configured
 * installations on rate-limit / permission refusals (the native
 * rotation's rule). Construction takes the parsed App configuration
 * and an optional fetch (tests inject a fake; the Worker uses the
 * platform fetch).
 */
export class AppRotationGitHubWriter implements GitHubWriter, GitHubItemSource {
  private readonly config: GitHubAppConfig;
  private readonly fetchFn: FetchFn;
  private readonly now: () => number;
  private appKey: Promise<CryptoKey> | null = null;
  private readonly tokens = new Map<number, CachedToken>();

  constructor(config: GitHubAppConfig, fetchFn?: FetchFn, now?: () => number) {
    this.config = config;
    this.fetchFn = fetchFn ?? ((url, init) => fetch(url, init));
    this.now = now ?? (() => Date.now());
  }

  private importAppKey(): Promise<CryptoKey> {
    if (!this.appKey) {
      this.appKey = crypto.subtle.importKey(
        "pkcs8",
        this.config.privateKeyPkcs8 as BufferSource,
        { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
        false,
        ["sign"],
      );
    }
    return this.appKey;
  }

  /** A short-lived App JWT (RS256) for the token exchange. */
  async mintAppJwt(): Promise<string> {
    const nowSec = Math.floor(this.now() / 1000);
    const header = { alg: "RS256", typ: "JWT" };
    const payload = {
      iss: this.config.appId,
      iat: nowSec - APP_JWT_BACKDATE_SECONDS,
      exp: nowSec + APP_JWT_TTL_SECONDS,
    };
    const input = `${b64urlEncode(JSON.stringify(header))}.${b64urlEncode(JSON.stringify(payload))}`;
    const signature = await crypto.subtle.sign(
      "RSASSA-PKCS1-v1_5",
      await this.importAppKey(),
      new TextEncoder().encode(input),
    );
    return `${input}.${b64urlEncode(new Uint8Array(signature))}`;
  }

  private async installationToken(installationId: number): Promise<string> {
    const cached = this.tokens.get(installationId);
    if (cached && cached.expiresAtMs - this.now() > TOKEN_REFRESH_MARGIN_MS) {
      return cached.token;
    }
    const jwt = await this.mintAppJwt();
    const res = await this.fetchFn(`${GITHUB_API}/app/installations/${installationId}/access_tokens`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${jwt}`,
        accept: "application/vnd.github+json",
        "x-github-api-version": "2022-11-28",
      },
    });
    if (res.status !== 201) {
      throw new FailClosed(
        502,
        "github_token_exchange_failed",
        `installation token exchange failed for installation ${installationId} (HTTP ${res.status})`,
      );
    }
    const body = (await res.json()) as { token?: unknown; expires_at?: unknown };
    if (typeof body.token !== "string" || typeof body.expires_at !== "string") {
      throw new FailClosed(
        502,
        "github_token_exchange_failed",
        `installation token exchange returned a malformed body for installation ${installationId}`,
      );
    }
    const expiresAtMs = Date.parse(body.expires_at);
    this.tokens.set(installationId, {
      token: body.token,
      expiresAtMs: Number.isNaN(expiresAtMs) ? 0 : expiresAtMs,
    });
    return body.token;
  }

  /** The native rotation's failover rule (#8887): rate-limit and
   * permission refusals move to the next token; other statuses are
   * real failures and surface as-is. */
  private isFailoverRefusal(status: number, headers: Headers, bodyText: string): boolean {
    if (status === 429) return true;
    if (status !== 403) return false;
    if (headers.get("x-ratelimit-remaining") === "0") return true;
    const lower = bodyText.toLowerCase();
    return (
      lower.includes("rate limit") ||
      lower.includes("secondary rate limit") ||
      lower.includes("resource not accessible by integration")
    );
  }

  async createItem(input: GitHubCreateInput): Promise<GitHubCreatedItem> {
    const path =
      input.kind === "pr" ? `/repos/${input.repo}/pulls` : `/repos/${input.repo}/issues`;
    const payload: Record<string, unknown> = { title: input.title, body: input.body };
    if (input.kind === "pr") {
      if (!input.head || !input.base) {
        throw new FailClosed(400, "invalid_body", "a PR create requires head and base branch claims");
      }
      payload["head"] = input.head;
      payload["base"] = input.base;
    }

    let lastStatus = 0;
    for (const installationId of this.config.installationIds) {
      const token = await this.installationToken(installationId);
      const res = await this.fetchFn(`${GITHUB_API}${path}`, {
        method: "POST",
        headers: {
          authorization: `Bearer ${token}`,
          accept: "application/vnd.github+json",
          "x-github-api-version": "2022-11-28",
          "content-type": "application/json",
        },
        body: JSON.stringify(payload),
      });
      if (res.status === 201) {
        const created = (await res.json()) as { number?: unknown; html_url?: unknown };
        if (typeof created.number !== "number" || typeof created.html_url !== "string") {
          throw new FailClosed(502, "github_create_failed", "GitHub create returned a malformed body");
        }
        return { number: created.number, url: created.html_url };
      }
      const bodyText = await res.text().catch(() => "");
      lastStatus = res.status;
      if (this.isFailoverRefusal(res.status, res.headers, bodyText)) {
        continue; // next installation in the rotation
      }
      throw new FailClosed(
        502,
        "github_create_failed",
        `GitHub create failed (HTTP ${res.status}) and is not a rotation failover case`,
      );
    }
    throw new FailClosed(
      502,
      "github_create_failed",
      `GitHub create refused by every installation in the rotation (last HTTP ${lastStatus})`,
    );
  }

  /** A GET through the rotation, under the same failover rule as
   * the writes. Returns the response on the first installation that
   * answers non-failover (200, 404, or a real failure, which
   * surfaces); throws when every installation refuses failover-style. */
  private async getThroughRotation(url: string): Promise<Response> {
    let lastStatus = 0;
    for (const installationId of this.config.installationIds) {
      const token = await this.installationToken(installationId);
      const res = await this.fetchFn(url, {
        method: "GET",
        headers: {
          authorization: `Bearer ${token}`,
          accept: "application/vnd.github+json",
          "x-github-api-version": "2022-11-28",
        },
      });
      if (res.status === 200 || res.status === 404) return res;
      const bodyText = await res.text().catch(() => "");
      lastStatus = res.status;
      if (this.isFailoverRefusal(res.status, res.headers, bodyText)) {
        continue; // next installation in the rotation
      }
      throw new FailClosed(
        502,
        "github_read_failed",
        `GitHub read failed (HTTP ${res.status}) and is not a rotation failover case`,
      );
    }
    throw new FailClosed(
      502,
      "github_read_failed",
      `GitHub read refused by every installation in the rotation (last HTTP ${lastStatus})`,
    );
  }

  /** The GitHub issues-API shape both reads consume: an entry is a
   * PR exactly when it carries the pull_request marker. */
  private toItem(repo: string, raw: Record<string, unknown>): GitHubItem | null {
    if (typeof raw["number"] !== "number" || typeof raw["html_url"] !== "string") return null;
    return {
      repo,
      kind: "pull_request" in raw ? "pr" : "issue",
      number: raw["number"],
      url: raw["html_url"],
      body: typeof raw["body"] === "string" ? raw["body"] : "",
    };
  }

  async getItem(repo: string, kind: "issue" | "pr", number: number): Promise<GitHubItem | null> {
    // The issues endpoint answers for PRs too (same object shape);
    // the kind the caller asked for is checked against the marker.
    const res = await this.getThroughRotation(`${GITHUB_API}/repos/${repo}/issues/${number}`);
    if (res.status === 404) return null;
    const raw = (await res.json()) as Record<string, unknown>;
    const item = this.toItem(repo, raw);
    if (!item || item.kind !== kind) return null;
    return item;
  }

  async listItems(repo: string): Promise<GitHubItem[]> {
    const out: GitHubItem[] = [];
    let url: string | null =
      `${GITHUB_API}/repos/${repo}/issues?state=all&per_page=100`;
    for (let page = 0; page < MAX_LIST_PAGES && url !== null; page++) {
      const res = await this.getThroughRotation(url);
      if (res.status === 404) return out;
      const rawList = (await res.json()) as unknown;
      if (!Array.isArray(rawList)) {
        throw new FailClosed(502, "github_read_failed", "GitHub list returned a malformed body");
      }
      for (const raw of rawList) {
        const item = this.toItem(repo, raw as Record<string, unknown>);
        if (item) out.push(item);
      }
      url = nextLinkUrl(res.headers.get("link"));
    }
    return out;
  }
}

/** Page cap for the fallback scan (100 items/page): a bound, stated
 * here, so a huge repo's scan terminates; ingestion (#12590) is the
 * completeness mechanism, the query fallback is a stopgap (§13). */
const MAX_LIST_PAGES = 10;

/** The rel="next" URL from a GitHub Link header, or null. */
function nextLinkUrl(linkHeader: string | null): string | null {
  if (!linkHeader) return null;
  for (const part of linkHeader.split(",")) {
    const match = /<([^>]+)>\s*;\s*rel="next"/.exec(part);
    if (match) return match[1];
  }
  return null;
}
