import subprocess, json, os, sys
OUT=os.path.expanduser("~/audit7-lane2")
os.makedirs(OUT, exist_ok=True)
repos=["developing-today/code","developing-today-forks/opencode","developing-today-forks/openchamber","dezren39/fleet"]
def api(path):
    out=subprocess.run(["fleet","gh","api",path,"--paginate"],capture_output=True,text=True,timeout=1200)
    if out.returncode!=0:
        print("ERR",path,out.stderr[-400:],flush=True); return []
    # paginate concatenates JSON arrays
    txt=out.stdout.strip()
    items=[]
    dec=json.JSONDecoder(); i=0
    while i < len(txt):
        while i < len(txt) and txt[i] in " \n\r\t": i+=1
        if i>=len(txt): break
        obj,j=dec.raw_decode(txt,i); i=j
        if isinstance(obj,list): items.extend(obj)
        else: items.append(obj)
    return items
for r in repos:
    slug=r.replace("/","__")
    issues=api(f"repos/{r}/issues?state=closed&since=2026-10-23T00:00:00Z&per_page=100")
    issues=[x for x in issues if x.get("pull_request") is None and (x.get("closed_at") or "")>="2026-10-23T00:00:00Z"]
    print(r, "issues", len(issues), flush=True)
    full=[]
    for n,iss in enumerate(issues):
        comments=api(f"repos/{r}/issues/{iss['number']}/comments?per_page=100")
        iss["_comments"]=[{"user":(c.get("user") or {}).get("login"),"created_at":c.get("created_at"),"body":c.get("body") or ""} for c in comments]
        full.append(iss)
        if n%25==0: print(" ",r,n,flush=True)
    with open(os.path.join(OUT,f"{slug}.json"),"w") as f: json.dump(full,f)
    print(r,"done",len(full),flush=True)
print("ALL DONE",flush=True)
