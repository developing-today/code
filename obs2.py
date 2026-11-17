# lane-scratch: any-a /tmp/lane-any-a-20261114-1306 task 4890
import sys, json, time, urllib.request
sys.path.insert(0, "/home/user/git/code/pm-stack/opencode-claimant")
import claimant

DIR = "/home/user/git/fleet/4890"
AGG = "http://127.0.0.1:4870"
EVID = {"steps": []}

def log(step, **kw):
    EVID["steps"].append({"t": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "step": step, **kw})
    print(step, json.dumps(kw)[:500], flush=True)

def agg(method, path, body=None):
    req = urllib.request.Request(AGG + path, method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"content-type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return r.status, json.loads(r.read().decode() or "{}")
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read().decode() or "{}")

def forms(sid):
    st, pl = claimant._server_request("GET", f"/api/session/{sid}/form", timeout=8)
    d = claimant._unwrap_envelope(pl)
    return st, d

def msgs(sid):
    st, pl = claimant._server_request("GET", f"/api/session/{sid}/message", timeout=8)
    d = claimant._unwrap_envelope(pl)
    return d if isinstance(d, list) else []

# 1. create session on a plan route that is not the walled OpenAI-Plus pool
body = {
    "title": "queue task 4890 - #1156 answer re-observation 2 (disposable)",
    "location": {"directory": DIR},
    "model": {"id": "deepseek-flash-plan", "providerID": "opencode-route"},
    "agent": "build",
}
status, payload = claimant._server_request("POST", "/api/session", body=body, timeout=15)
sess = claimant._unwrap_envelope(payload)
sid = sess.get("id") if isinstance(sess, dict) else None
log("session_created", status=status, session=sid, model="opencode-route/deepseek-flash-plan")
if not sid:
    print("FAIL no session", json.dumps(payload)[:600]); sys.exit(1)
EVID["session_id"] = sid
with open(DIR + "/.obs-session2.json", "w") as f:
    json.dump({"session_id": sid}, f)

# 2. prompt it to ask the marker question
prompt = (
    "This is a controlled round-trip test (queue task 4890, issue #1156). "
    "Use the question tool NOW to ask exactly this question: "
    "'Which marker word should be recorded for the answer round-trip, ALPHA or BETA?' "
    "with two options: ALPHA and BETA. Do not answer it yourself, do not pick a default, "
    "and do no other work. After the question is answered, reply with exactly: "
    "ANSWER RECORDED: <the answer you received> - and nothing else."
)
ok = claimant._send_prompt(sid, prompt)
log("prompt_delivered", ok=ok)

# 3. poll for a pending form (the question tool's form), up to ~300s, one nudge allowed
form = None
nudged = False
deadline = time.time() + 300
while time.time() < deadline:
    time.sleep(5)
    st, fl = forms(sid)
    items = fl if isinstance(fl, list) else (fl.get("forms") if isinstance(fl, dict) else None) or []
    for it in items:
        if isinstance(it, dict):
            log("form_seen", form=json.dumps(it)[:400])
            form = it
            break
    if form:
        break
    ms = msgs(sid)
    types = [m.get("type") for m in ms[-4:]] if ms else []
    if not nudged and types and types[-1] == "idle" and time.time() > deadline - 200:
        claimant._send_prompt(sid, "Retry: use your question tool to ask me exactly this question, then wait for my answer: Which marker word should be recorded for the answer round-trip, ALPHA or BETA? Do not do anything else.")
        nudged = True
        log("nudged", tail_types=types)
if not form:
    log("no_form_within_300s", messages=json.dumps(msgs(sid))[-1500:])
    print("EVIDENCE", json.dumps(EVID)[:2000]); sys.exit(2)
form_id = form.get("id") or form.get("formID")
EVID["form"] = form
log("form_pending", form_id=form_id)

# 4. post the dashboard answer action (BETA) via the aggregator actions path
abody = {
    "kind": "session", "verb": "answer",
    "objectRef": f"session:{sid}", "sessionId": sid,
    "sessionSource": "opencode-laptop",
    "payload": {"text": "BETA"},
    "author": {"kind": "agent", "id": "repo-worker-code", "label": "lane any-a task 4890 re-observation"},
    "clientToken": f"task4890-reobs-{int(time.time())}",
}
st, resp = agg("POST", "/api/actions", abody)
act = resp.get("action") or resp
aid = act.get("id") if isinstance(act, dict) else None
EVID["action_post_status"] = st; EVID["action"] = act
log("answer_action_posted", status=st, action_id=aid, resp=json.dumps(resp)[:300])
if not aid:
    print("EVIDENCE", json.dumps(EVID)[:2000]); sys.exit(3)
t_post = time.time()

# 5. poll the action until applied/rejected (consumer is minutely), up to 300s
action_final = None
while time.time() - t_post < 300:
    time.sleep(10)
    st, allact = agg("GET", "/api/actions.json")
    for a in (allact.get("actions") or []):
        if a.get("id") == aid:
            action_final = a
            if a.get("state") != "new":
                break
    if action_final and action_final.get("state") != "new":
        break
    log("action_waiting", state=(action_final or {}).get("state"))
EVID["action_final"] = action_final
log("action_final", action=json.dumps(action_final)[:600], waited_s=int(time.time() - t_post))

# 6. poll session for the question completing with BETA, up to 300s
verdict = None
while time.time() - t_post < 420:
    time.sleep(10)
    ms = msgs(sid)
    blob = json.dumps(ms)
    st, fl = forms(sid)
    if "ANSWER RECORDED: BETA" in blob:
        verdict = "answer_recorded_beta"; break
    if "BETA" in blob and form_id and "timed out" not in blob and "not answered" not in blob:
        # form may have completed; check form state text
        if "answer" in blob.lower():
            verdict = "beta_present_with_answer"; break
    log("session_waiting", forms=json.dumps(fl)[:200], tail=blob[-300:])
EVID["verdict"] = verdict
EVID["final_forms"] = fl if 'fl' in dir() else None
EVID["final_messages_tail"] = json.dumps(msgs(sid))[-2500:]
log("verdict", verdict=verdict)
print("EVIDENCE_JSON_BEGIN")
print(json.dumps(EVID, indent=1)[:6000])
print("EVIDENCE_JSON_END")
