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

function derLength(n: number): number[] {
  if (n < 0x80) return [n];
  const bytes: number[] = [];
  let v = n;
  while (v > 0) {
    bytes.unshift(v & 0xff);
    v >>= 8;
  }
  return [0x80 | bytes.length, ...bytes];
}

/**
 * Wrap a PKCS#1 RSAPrivateKey DER in the PKCS#8 PrivateKeyInfo envelope.
 * GitHub issues App private keys as PKCS#1 ("BEGIN RSA PRIVATE KEY"), but
 * WebCrypto imports only PKCS#8 — without this wrap every key GitHub hands
 * out fails importKey with "Invalid keyData".
 */
export function wrapPkcs1InPkcs8(pkcs1: Uint8Array): Uint8Array {
  // AlgorithmIdentifier: SEQUENCE { OID rsaEncryption (1.2.840.113549.1.1.1), NULL }
  const algId = [0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00];
  const version = [0x02, 0x01, 0x00]; // INTEGER 0
  const octetString = [0x04, ...derLength(pkcs1.length), ...pkcs1];
  const body = [...version, ...algId, ...octetString];
  return new Uint8Array([0x30, ...derLength(body.length), ...body]);
}

async function importRsaKey(der: Uint8Array): Promise<CryptoKey> {
  const params = { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" } as const;
  try {
    return await crypto.subtle.importKey("pkcs8", der as unknown as BufferSource, params, false, ["sign"]);
  } catch {
    return await crypto.subtle.importKey("pkcs8", wrapPkcs1InPkcs8(der) as unknown as BufferSource, params, false, [
      "sign",
    ]);
  }
}

export async function appJwt(env: Record<string, string | undefined>): Promise<string> {
  const appId = env.GITHUB_APP_ID;
  const rawKey = env.GITHUB_APP_PRIVATE_KEY;
  if (!appId || !rawKey) throw new Error("github_app_credentials_missing");
  const b64 = decodePem(rawKey);
  const der = Uint8Array.from(Buffer.from(b64, "base64"));
  const key = await importRsaKey(der);
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

const discoveredInstallations = new Map<string, number>();

/**
 * Resolve the installation id for one repo via GET /repos/{repo}/installation
 * (App JWT). An app installed on several accounts (an org + a user) has one
 * installation per account, so a single static id cannot cover every repo.
 * Results are cached per process.
 */
async function discoverInstallationId(
  env: Record<string, string | undefined>,
  repo: string,
  fetchFn: typeof fetch,
): Promise<number | undefined> {
  const cached = discoveredInstallations.get(repo);
  if (cached) return cached;
  const jwt = await appJwt(env);
  const res = await fetchFn(`https://api.github.com/repos/${repo}/installation`, {
    headers: { authorization: `Bearer ${jwt}`, accept: "application/vnd.github+json", "user-agent": "pm-projector" },
  });
  if (!res.ok) return undefined;
  const body = (await res.json()) as { id?: number };
  if (typeof body.id === "number") {
    discoveredInstallations.set(repo, body.id);
    return body.id;
  }
  return undefined;
}

/** Resolve an API token for a repo: installation token (app) or plain token. */
export async function apiTokenFor(
  env: Record<string, string | undefined>,
  repo: string,
  fetchFn: typeof fetch = fetch,
  installationIds?: Record<string, number>,
): Promise<GitHubAuth> {
  let installId = installationIds?.[repo];
  if (!installId && env.GITHUB_APP_ID && env.GITHUB_APP_PRIVATE_KEY) {
    installId = await discoverInstallationId(env, repo, fetchFn);
  }
  if (!installId && env.GITHUB_APP_INSTALL_ID) installId = Number(env.GITHUB_APP_INSTALL_ID);
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
    // nextPageFromLink returns the Link header's absolute URL — use it
    // as-is. Prefixing it again produced api.github.com/https://... 404s,
    // which discarded every multi-page stream (written=0 everywhere).
    const url = path.startsWith("http") ? path : `https://api.github.com/${path}`;
    const res = await fetchFn(url, {
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
