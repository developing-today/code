// Minimal GitHub REST client for the queue transport alert (code#6369).
// Each method throws on a non-2xx response so the caller can keep the
// window un-alerted (or un-recovered) and retry on the next cron tick.
import { INCIDENT_LABELS, INCIDENT_TITLE_PREFIX } from "./logic.js";

const API = "https://api.github.com";

export function makeGitHub(token, repo, fetchImpl = fetch) {
  async function call(method, path, body) {
    const res = await fetchImpl(`${API}${path}`, {
      method,
      headers: {
        Authorization: `Bearer ${token}`,
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "queue-transport-alert-worker",
        ...(body ? { "Content-Type": "application/json" } : {}),
      },
      body: body ? JSON.stringify(body) : undefined,
    });
    const text = await res.text();
    if (!res.ok) {
      throw new Error(`GitHub ${method} ${path} -> HTTP ${res.status}: ${text.slice(0, 200)}`);
    }
    return text ? JSON.parse(text) : null;
  }

  return {
    /** Number of an open incident issue, or null. */
    async find() {
      const q = `repo:${repo} is:issue is:open in:title "${INCIDENT_TITLE_PREFIX}"`;
      const data = await call("GET", `/search/issues?q=${encodeURIComponent(q)}&per_page=20`);
      const hit = (data.items || []).find((i) => String(i.title || "").startsWith(INCIDENT_TITLE_PREFIX));
      return hit ? hit.number : null;
    },
    async file(title, body) {
      const issue = await call("POST", `/repos/${repo}/issues`, { title, body, labels: INCIDENT_LABELS });
      return issue.number;
    },
    async comment(number, body) {
      await call("POST", `/repos/${repo}/issues/${number}/comments`, { body });
    },
    async close(number) {
      await call("PATCH", `/repos/${repo}/issues/${number}`, { state: "closed", state_reason: "completed" });
    },
  };
}
