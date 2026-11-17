# lane-scratch: any-a /tmp/lane-any-a-20261114-1306 task 4890
import sys, json, time
sys.path.insert(0, "/home/user/git/code/pm-stack/opencode-claimant")
import claimant

DIR = "/home/user/git/fleet/4890"
STATE = DIR + "/.obs-session.json"

body = {
    "title": "queue task 4890 - #1156 answer re-observation (disposable)",
    "location": {"directory": DIR},
    "model": {"id": "workhorse", "providerID": "opencode-route"},
    "agent": "build",
}
status, payload = claimant._server_request("POST", "/api/session", body=body, timeout=15)
print("create status", status)
sess = claimant._unwrap_envelope(payload)
sid = sess.get("id") if isinstance(sess, dict) else None
print("session", sid)
if not sid:
    print("create payload", json.dumps(payload)[:800]); sys.exit(1)
with open(STATE, "w") as f:
    json.dump({"session_id": sid, "created": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}, f)

prompt = (
    "This is a controlled round-trip test (queue task 4890, issue #1156). "
    "Use the question tool NOW to ask exactly this question: "
    "'Which marker word should be recorded for the answer round-trip, ALPHA or BETA?' "
    "with two options: ALPHA and BETA. Do not answer it yourself, do not pick a default, "
    "and do no other work. After the question is answered, reply with exactly: "
    "ANSWER RECORDED: <the answer you received> - and nothing else."
)
ok = claimant._send_prompt(sid, prompt)
print("prompt delivered", ok)

def question_state():
    found = []
    for path in ("/api/question", "/question"):
        st, pl = claimant._server_request("GET", path, timeout=5)
        if claimant._is_2xx(st):
            data = claimant._unwrap_envelope(pl)
            if isinstance(data, list):
                for q in data:
                    if isinstance(q, dict) and q.get("sessionID") == sid or (isinstance(q, dict) and q.get("sessionId") == sid):
                        found.append(q)
            elif isinstance(data, dict) and data:
                found.append({"raw": data})
    return found

for i in range(30):
    time.sleep(10)
    qs = question_state()
    msgs = claimant._get_messages(sid)
    summary = []
    if msgs:
        for m in msgs[-3:]:
            parts = m.get("parts") or []
            for p in parts:
                t = p.get("type")
                if t == "tool":
                    summary.append({"tool": p.get("tool"), "state": (p.get("state") or {}).get("status")})
                elif t == "text":
                    summary.append({"text": (p.get("text") or "")[:80]})
    print(f"poll {i} pending_questions={json.dumps(qs)[:400]} tail={json.dumps(summary)[:400]}", flush=True)
    if qs:
        print("QUESTION_PENDING", json.dumps(qs)[:1000])
        break
else:
    print("NO_QUESTION_WITHIN_300S")
