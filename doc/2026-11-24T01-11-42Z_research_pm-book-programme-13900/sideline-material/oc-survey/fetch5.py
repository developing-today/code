import subprocess, json, os, sys, urllib.parse
FLEET=os.path.expanduser("~/tools/bin/fleet")
OUT=os.path.expanduser("~/.oc-survey")
def run(p, jq):
    try:
        r=subprocess.run([FLEET,"gh","api",p,"--jq",jq],capture_output=True,text=True,timeout=120)
    except subprocess.TimeoutExpired:
        print(f"TIMEOUT {p}",flush=True); return []
    if r.returncode!=0:
        print(f"FAIL {p}: {r.stderr[:200]}",flush=True); return []
    out=[]
    for l in r.stdout.strip().splitlines():
        if not l.strip(): continue
        try:
            v=json.loads(l)
            if isinstance(v,str): v=json.loads(v)
            out.append(v)
        except Exception: pass
    return out
def search(q, name):
    enc=urllib.parse.quote(q)
    t=run(f"search/issues?q={enc}&per_page=1", '.total_count | @json')
    print(name,"total",t,flush=True)
    out=[]; page=1
    while page<=10:
        chunk=run(f"search/issues?q={enc}&per_page=100&page={page}", '.items[] | {number,title,state,created_at,updated_at,closed_at,user:.user.login,labels:[.labels[].name],html_url,is_pr:(has("pull_request")),body:((.body // "")[0:1500])} | @json')
        if not chunk: break
        out.extend(chunk)
        if len(chunk)<100: break
        page+=1
    with open(os.path.join(OUT,name),"w") as f: json.dump(out,f)
    print(name,len(out),flush=True)
    return out
search("repo:developing-today/code opencode in:title","code-title-opencode.json")
search("repo:developing-today/code openchamber in:title","code-title-openchamber.json")
search("repo:developing-today/code claimant in:title","code-title-claimant.json")
search("repo:developing-today/code opencode is:issue is:open","code-open-opencode.json")
search("repo:developing-today/code openchamber is:issue is:open","code-open-openchamber.json")
# individually referenced numbers in code
nums=[1344,1357,1363,1338,1187,3026,4757,4758,4870,4912,4916,5016,5060,7325,7356,7366,7367,7375,7706,7732,7760,7770,7772,8097,8163,7991,2072,1858,2841,3286,730,13675,8882,6129,359]
res=[]
for n in nums:
    v=run(f"repos/developing-today/code/issues/{n}", '{number,title,state,created_at,updated_at,closed_at,user:.user.login,labels:[.labels[].name],html_url,is_pr:(has("pull_request")),body:((.body // "")[0:1500])} | @json')
    if v: res.append(v[0]); print("code#",n,v[0]['title'][:80],v[0]['state'],flush=True)
with open(os.path.join(OUT,"code-referenced.json"),"w") as f: json.dump(res,f)
print("referenced",len(res),flush=True)
