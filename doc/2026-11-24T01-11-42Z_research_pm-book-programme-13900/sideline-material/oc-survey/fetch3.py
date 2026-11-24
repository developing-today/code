import subprocess, json, os, sys, urllib.parse

FLEET = os.path.expanduser("~/tools/bin/fleet")
OUT = os.path.expanduser("~/.oc-survey")


def run(p, jq):
    r = subprocess.run(
        [FLEET, "gh", "api", p, "--jq", jq], capture_output=True, text=True, timeout=180
    )
    if r.returncode != 0:
        print(f"FAIL {p}: {r.stderr[:400]}", file=sys.stderr, flush=True)
        return []
    out = []
    for l in r.stdout.strip().splitlines():
        if not l.strip():
            continue
        try:
            v = json.loads(l)
            if isinstance(v, str):
                v = json.loads(v)
            out.append(v)
        except Exception as e:
            print("parse", e, l[:100], flush=True)
    return out


def api_jq(path, jq):
    out = []
    page = 1
    while True:
        sep = "&" if "?" in path else "?"
        chunk = run(f"{path}{sep}per_page=100&page={page}", jq)
        if not chunk:
            break
        out.extend(chunk)
        if len(chunk) < 100:
            break
        page += 1
    return out


def save(name, data):
    with open(os.path.join(OUT, name), "w") as f:
        json.dump(data, f)
    print(name, len(data), flush=True)


PRJQ = ".[] | {number,title,state,merged_at,created_at,closed_at,updated_at,user:.user.login,labels:[.labels[].name],html_url,base:.base.ref,head:.head.ref} | @json"
ISSJQ = '.[] | {number,title,state,created_at,updated_at,closed_at,user:.user.login,labels:[.labels[].name],html_url,milestone:(.milestone.title // null),is_pr:(has("pull_request")),body:(.body // "")} | @json'
for repo, short in [
    ("developing-today-forks/opencode", "oc"),
    ("developing-today-forks/openchamber", "och"),
]:
    save(f"pulls-closed-{short}.json", api_jq(f"repos/{repo}/pulls?state=closed", PRJQ))
    save(f"pulls-open-{short}.json", api_jq(f"repos/{repo}/pulls?state=open", PRJQ))
    save(f"issues-open-{short}.json", api_jq(f"repos/{repo}/issues?state=open", ISSJQ))
    save(
        f"issues-closed-{short}.json",
        api_jq(f"repos/{repo}/issues?state=closed", ISSJQ),
    )
    save(
        f"branches-{short}.json",
        api_jq(f"repos/{repo}/branches", ".[] | {name} | @json"),
    )
    save(f"labels-{short}.json", api_jq(f"repos/{repo}/labels", ".[] | {name} | @json"))


# code repo: search
def search(q):
    enc = urllib.parse.quote(q)
    out = []
    page = 1
    total = None
    while True:
        chunk_raw = run(
            f"search/issues?q={enc}&per_page=100&page={page}",
            '.items[] | {number,title,state,created_at,updated_at,closed_at,user:.user.login,labels:[.labels[].name],html_url,is_pr:(has("pull_request")),body:(.body // "")} | @json',
        )
        if total is None:
            t = run(f"search/issues?q={enc}&per_page=1", ".total_count | @json")
            total = t[0] if t else None
            print("search total", q, total, flush=True)
        if not chunk_raw:
            break
        out.extend(chunk_raw)
        if len(chunk_raw) < 100:
            break
        page += 1
        if page > 10:
            break
    return out


for qname, q in [
    ("code-search-opencode", "repo:developing-today/code opencode"),
    ("code-search-openchamber", "repo:developing-today/code openchamber"),
]:
    save(f"{qname}.json", search(q))
