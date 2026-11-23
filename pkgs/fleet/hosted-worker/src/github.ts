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

/** Fixture writer: records every created item in memory (bodies read
 * back exactly as written, which is how the endpoint's acceptance is
 * checked locally) and numbers items per repo, issues and PRs in one
 * sequence as GitHub does. */
export class FixtureGitHubWriter implements GitHubWriter {
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
export class AppRotationGitHubWriter implements GitHubWriter {
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
}
