// GitHub API access for reconciliation reads. The projector self-heals beyond
// webhook delivery by polling GitHub with the GitHub App's credentials.
//
// Credentials come from the environment, referenced by NAME only:
//   GITHUB_APP_ID            the app's numeric id
//   GITHUB_APP_PRIVATE_KEY   PEM (or base64 of PEM) private key
//   GITHUB_APP_INSTALL_ID    the installation id on the app (per repo, optional)
//   GITHUB_TOKEN             fallback: a plain token for repos the app can't reach
//
// Nothing here writes back to GitHub — read-only REST calls only.

const b64url = (bytes: Uint8Array): string =>
  Buffer.from(bytes).toString("base64").replace(/=/g, "").replace(/\+/g, "-").replace(/\//g, "_");
const encoder = new TextEncoder();

function decodePem(pem: string): string {
  const trimmed = pem.replace(/-----[^-]+-----/g, "").replace(/\s/g, "");
  if (/^[A-Za-z0-9+/]+=*$/.test(trimmed) && !trimmed.includes("PRIVATE")) {
    // already base64 of the DER key
    return trimmed;
  }
  return trimmed;
}

export async function appJwt(env: Record<string, string | undefined>): Promise<string> {
  const appId = env.GITHUB_APP_ID;
  const rawKey = env.GITHUB_APP_PRIVATE_KEY;
  if (!appId || !rawKey) throw new Error("github_app_credentials_missing");
  const b64 = decodePem(rawKey);
  const der = Uint8Array.from(Buffer.from(b64, "base64"));
  const key = await crypto.subtle.importKey(
    "pkcs8",
    der as unknown as BufferSource,
    { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const now = Math.floor(Date.now() / 1000);
  const header = { alg: "RS256", typ: "JWT" };
  const claims = { iat: now - 60, exp: now + 540, iss: appId };
  const unsigned = [header, claims].map((o) => b64url(encoder.encode(JSON.stringify(o)))).join(".");
  const sig = new Uint8Array(await crypto.subtle.sign("RSASSA-PKCS1-v1_5", key, encoder.encode(unsigned)));
  return `${unsigned}.${b64url(sig)}`;
}

export interface GitHubAuth {
  kind: "app" | "token";
  token: string;
}

/** Resolve an API token for a repo: installation token (app) or plain token. */
export async function apiTokenFor(
  env: Record<string, string | undefined>,
  repo: string,
  fetchFn: typeof fetch = fetch,
  installationIds?: Record<string, number>,
): Promise<GitHubAuth> {
  const installId =
    installationIds?.[repo] ?? (env.GITHUB_APP_INSTALL_ID ? Number(env.GITHUB_APP_INSTALL_ID) : undefined);
  if (env.GITHUB_APP_ID && env.GITHUB_APP_PRIVATE_KEY && installId) {
    const jwt = await appJwt(env);
    const res = await fetchFn(`https://api.github.com/app/installations/${installId}/access_tokens`, {
      method: "POST",
      headers: { authorization: `Bearer ${jwt}`, accept: "application/vnd.github+json" },
    });
    if (!res.ok) throw new Error(`installation_token_failed_${res.status}`);
    const body = (await res.json()) as { token: string };
    return { kind: "app", token: body.token };
  }
  if (env.GITHUB_TOKEN) return { kind: "token", token: env.GITHUB_TOKEN };
  throw new Error(`no_github_credentials_for_${repo}`);
}

async function ghJson<T>(auth: GitHubAuth, path: string, fetchFn: typeof fetch): Promise<T> {
  const res = await fetchFn(`https://api.github.com/${path}`, {
    headers: {
      authorization: `Bearer ${auth.token}`,
      accept: "application/vnd.github+json",
      "user-agent": "pm-projector",
    },
  });
  if (res.status === 404) throw new Error("not_found");
  if (!res.ok) throw new Error(`github_api_${res.status}`);
  return (await res.json()) as T;
}

/** Extract Link rel="next" from the Link header. */
export function nextPageFromLink(linkHeader: string | null): string | null {
  if (!linkHeader) return null;
  for (const part of linkHeader.split(",")) {
    const match = part.match(/<([^>]+)>;\s*rel="next"/);
    if (match) return match[1];
  }
  return null;
}

export async function listResource<T>(auth: GitHubAuth, initialPath: string, fetchFn: typeof fetch): Promise<T[]> {
  const out: T[] = [];
  let path: string | null = initialPath;
  while (path) {
    const res = await fetchFn(`https://api.github.com/${path}`, {
      headers: {
        authorization: `Bearer ${auth.token}`,
        accept: "application/vnd.github+json",
        "user-agent": "pm-projector",
      },
    });
    if (!res.ok) throw new Error(`github_api_${res.status}`);
    const page = (await res.json()) as T[];
    out.push(...page);
    path = nextPageFromLink(res.headers.get("link"));
  }
  return out;
}
