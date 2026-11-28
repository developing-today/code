// Files the post-outage triage task into the fleet work queue through
// the broker's validated /enqueue verb (the same verb and repo-task/v1
// packet shape pm-dispatch uses, pm-stack/dispatch/src/packet.ts). The
// task is anchored to the GitHub incident, so its dedupe key is the
// canonical triage key for that issue and a retry cannot double-file.

export function buildOutagePacket(item, repo, nowMs) {
  const now = new Date(nowMs).toISOString();
  const url = `https://github.com/${repo}/issues/${item.incident}`;
  const window = `${item.start} to ${item.end}`;
  const prompt = [
    `The repo-queue broker was unreachable from ${window} (queue transport alert, code#6369).`,
    `Incident: ${url}.`,
    "Find out why: broker host off, tailscaled stopped, broker process down, or the Cloudflare tunnel.",
    "Record the cause on the incident. If a fix or guard is needed, open it or file a follow-up issue.",
    "Do not close the incident; a human closes it (#10136).",
  ].join("\n");
  return {
    title: `Queue broker outage ${window}: find the cause (#${item.incident})`,
    body: prompt,
    priority: 20,
    dedupe_key: `repo-health:v1:triage:${repo}:issue:${item.incident}`,
    payload: {
      schema: "repo-task/v1",
      kind: "triage",
      repo,
      problem: `The repo-queue broker was unreachable from ${window}; lanes starved at their health gate.`,
      evidence: [
        `Queue transport alert Worker: consecutive failed probes of the broker's public /healthz from ${window}`,
        `Incident issue: ${url}`,
      ],
      acceptance: [
        "The outage's cause is identified and recorded on the incident issue",
        "A fix PR or follow-up issue exists if the cause needs one",
      ],
      scope: {
        include: [`${repo}: the broker host, tailscale, the broker service and the pm-stack tunnel`],
        exclude: ["Closing the incident (a human closes it, #10136)", "Unrelated refactors"],
      },
      tests: ["None unless a fix lands; then the touched area's standard checks"],
      risk: { level: "low", flags: [] },
      priority_reason: "A queue outage starves every lane; knowing why prevents the next one.",
      timebox_minutes: 20,
      max_attempts: 1000,
      context_refs: [url],
      dedupe: { checked_at: now, basis: `Anchored to incident issue #${item.incident}; canonical triage key.` },
      issue: { number: item.incident, url, state_checked_at: now },
      origin: "queue-transport-alert",
      prompt,
    },
  };
}

export function makeBrokerEnqueuer({ brokerUrl, agentToken, accessId, accessSecret, queue, repo }, fetchImpl = fetch, clock = Date.now) {
  return async function enqueue(item) {
    const packet = buildOutagePacket(item, repo, clock());
    const res = await fetchImpl(`${brokerUrl.replace(/\/+$/, "")}/enqueue`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-Agent-Token": agentToken,
        "CF-Access-Client-Id": accessId,
        "CF-Access-Client-Secret": accessSecret,
        "User-Agent": "queue-transport-alert-worker",
      },
      body: JSON.stringify({ packet, queue }),
      redirect: "manual",
      signal: AbortSignal.timeout(15000),
    });
    const text = await res.text();
    let env = null;
    try {
      env = JSON.parse(text);
    } catch {
      env = null;
    }
    if (res.ok && env && env.ok !== false) return;
    // An existing task with the same dedupe key is the outcome we want.
    if (env && env.ok === false && /dedupe|duplicate|already/i.test(String(env.error || ""))) return;
    throw new Error(`broker /enqueue -> HTTP ${res.status}: ${text.slice(0, 200)}`);
  };
}
