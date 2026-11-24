#!/usr/bin/env python3
"""Decision eval runner (code#13675 Part 3b, Worker C).
Calls OpenAI (Luna low + harness) and Google Decisions (native baseline)
directly. Keys fetched by name via `fleet secrets get`, held in memory only.
Grades deterministically per harness.md. Writes raw outputs + results.json.
"""
import json, subprocess, sys, urllib.request, urllib.error, pathlib

HERE = pathlib.Path(__file__).parent
SYSTEM = ("You are a decision engine, not a chat assistant. You receive a STATE and one "
  "typed QUESTION. Answer with exactly one JSON object and nothing else, in the "
  "contract shape for the question type:\n"
  "- noul (yes/no): {\"value\": true or false, \"confidence\": <number 0 to 1>}\n"
  "- choice: {\"value\": \"<exactly one of the listed option ids>\", \"confidence\": <number 0 to 1>}\n"
  "- score: {\"value\": <integer level index, 0-based, lowest first>, \"confidence\": <number 0 to 1>}\n"
  "Rules: apply any rule or threshold stated in the STATE literally. Pick the "
  "single best-matching option or level from the criteria given. Never invent an "
  "option id or a level outside the given range. Output no prose, no markdown, "
  "no explanation - only the JSON object.")

def get_secret(name):
    r = subprocess.run(["/home/hatch/tools/fleet", "secrets", "get", name],
                       capture_output=True, text=True, timeout=120)
    if r.returncode != 0:
        raise RuntimeError(f"secret fetch failed for {name}: {r.stderr.strip()[:200]}")
    return r.stdout.strip()

def post(url, headers, body, timeout=90):
    req = urllib.request.Request(url, data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json", **headers},
                                 method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        try: payload = json.loads(e.read().decode())
        except Exception: payload = {"error": "unparseable error body"}
        return e.code, payload

def value_schema(item):
    q = item["question"]
    if item["family"] == "noul": return {"type": "boolean"}
    if item["family"] == "choice": return {"type": "string", "enum": list(q["criteria"].keys())}
    return {"type": "integer", "minimum": 0, "maximum": len(q["criteria"]) - 1}

def user_msg(item):
    q = item["question"]
    lines = [f"STATE:\n{item['state']}", f"\nQUESTION ({q['type']}): {q['instructions']}"]
    if q.get("criteria"):
        if isinstance(q["criteria"], dict):
            lines.append("OPTIONS:\n" + "\n".join(f"- {k}: {v}" for k, v in q["criteria"].items()))
        else:
            lines.append("LEVELS (lowest first, index from 0):\n" + "\n".join(f"- {i}: {v}" for i, v in enumerate(q["criteria"])))
    return "\n".join(lines)

def run_luna(items, key):
    out, tin, tout = [], 0, 0
    for it in items:
        schema = {"type": "object",
                  "properties": {"value": value_schema(it),
                                 "confidence": {"type": "number", "minimum": 0, "maximum": 1}},
                  "required": ["value", "confidence"], "additionalProperties": False}
        body = {"model": "gpt-6-luna", "reasoning_effort": "low",
                "messages": [{"role": "system", "content": SYSTEM},
                             {"role": "user", "content": user_msg(it)}],
                "response_format": {"type": "json_schema",
                                    "json_schema": {"name": "decision", "strict": True, "schema": schema}}}
        status, payload = post("https://api.openai.com/v1/chat/completions",
                               {"Authorization": f"Bearer {key}"}, body)
        rec = {"id": it["id"], "http": status, "raw": payload}
        u = payload.get("usage") or {}
        tin += u.get("prompt_tokens", 0) or 0; tout += u.get("completion_tokens", 0) or 0
        out.append(rec)
        print(f"luna {it['id']} http={status}", flush=True)
    return out, tin, tout

def run_google(items, key):
    out = []
    for it in items:
        body = {"model": "gemini-decision-1", "input": it["state"],
                "questions": {"q": it["question"]}}
        status, payload = post("https://generativelanguage.googleapis.com/v1beta/decisions",
                               {"x-goog-api-key": key}, body)
        out.append({"id": it["id"], "http": status, "raw": payload})
        print(f"google {it['id']} http={status}", flush=True)
    return out

def grade_luna(item, rec):
    if rec["http"] != 200: return False, None
    try:
        content = rec["raw"]["choices"][0]["message"]["content"]
        obj = json.loads(content)
    except Exception: return False, None
    if set(obj.keys()) != {"value", "confidence"}: return False, None
    v, c = obj["value"], obj["confidence"]
    if not isinstance(c, (int, float)) or isinstance(c, bool) or not (0 <= c <= 1): return False, None
    fam = item["family"]
    if fam == "noul" and not isinstance(v, bool): return False, None
    if fam == "choice" and v not in item["question"]["criteria"]: return False, None
    if fam == "score" and (not isinstance(v, int) or isinstance(v, bool)
                           or not (0 <= v < len(item["question"]["criteria"]))): return False, None
    return True, v

def grade_google(item, rec):
    if rec["http"] != 200: return False, None
    ans = (rec["raw"].get("answers") or {}).get("q")
    if not isinstance(ans, dict): return False, None
    fam = item["family"]
    if fam == "noul":
        if "noul" not in ans or "confidence" not in ans: return False, None
        return True, bool(ans["noul"] >= 0.5)
    if fam == "choice":
        if "choice" not in ans or "confidence" not in ans: return False, None
        return True, ans["choice"]
    probs = ans.get("probabilities")
    if not isinstance(probs, dict) or not probs: return False, None
    best = max(probs, key=lambda k: probs[k])
    try: return True, int(best)
    except Exception:
        legend = ans.get("legend") or {}
        for k, label in legend.items():
            if label == best: return True, int(k)
        return False, None

def correct(item, value):
    if value is None: return False
    if item["family"] == "score": return abs(value - item["expected"]) <= 1
    return value == item["expected"]

def summarise(items, graded):
    fams = {}
    for fam in ("noul", "choice", "score"):
        rows = [g for i, g in zip(items, graded) if i["family"] == fam]
        fams[fam] = {"n": len(rows),
                     "schema_faithful": sum(1 for g in rows if g[0]),
                     "correct": sum(1 for g in rows if g[1])}
    return {"families": fams,
            "schema_faithful": sum(1 for g in graded if g[0]),
            "correct": sum(1 for g in graded if g[1]), "n": len(graded)}

def main():
    data = json.loads((HERE / "eval-set.json").read_text())
    items = data["items"]
    openai_key = get_secret("openai.payg-api-key")
    google_key = get_secret("google.gemini-api-key")
    luna_raw, tin, tout = run_luna(items, openai_key)
    google_raw = run_google(items, google_key)
    (HERE / "raw-luna.json").write_text(json.dumps(luna_raw, indent=2))
    (HERE / "raw-google.json").write_text(json.dumps(google_raw, indent=2))
    luna_graded, google_graded = [], []
    detail = []
    for it, lr, gr in zip(items, luna_raw, google_raw):
        ls, lv = grade_luna(it, lr); gs, gv = grade_google(it, gr)
        lc, gc = correct(it, lv), correct(it, gv)
        luna_graded.append((ls, lc)); google_graded.append((gs, gc))
        detail.append({"id": it["id"], "family": it["family"], "expected": it["expected"],
                       "luna": {"schema": ls, "value": lv, "correct": lc, "http": lr["http"]},
                       "google": {"schema": gs, "value": gv, "correct": gc, "http": gr["http"]}})
    results = {"luna": summarise(items, luna_graded), "google": summarise(items, google_graded),
               "luna_tokens": {"prompt": tin, "completion": tout, "total": tin + tout},
               "detail": detail}
    (HERE / "results.json").write_text(json.dumps(results, indent=2))
    print(json.dumps({k: results[k] for k in ("luna", "google", "luna_tokens")}, indent=2))

if __name__ == "__main__":
    main()
