import subprocess, json, os, urllib.parse
OUT=os.path.expanduser("~/audit7-lane2")
repos=["developing-today/code","developing-today-forks/opencode","developing-today-forks/openchamber","dezren39/fleet"]
windows=[("2026-10-23","2026-10-24"),("2026-10-25","2026-10-26"),("2026-10-27","2026-10-28"),("2026-10-29","2026-10-30"),("2026-10-31","2026-11-01"),("2026-11-02","2026-11-03"),("2026-11-04","2026-11-05"),("2026-11-06","2026-11-07"),("2026-11-08","2026-11-09"),("2026-11-10","2026-11-11"),("2026-11-12","2026-11-13"),("2026-11-14","2026-11-15"),("2026-11-16","2026-11-17"),("2026-11-18","2026-11-19"),("2026-11-20","2026-11-21"),("2026-11-22","2026-11-23")]
def api(path):
    out=subprocess.run(["fleet","gh","api",path,"--paginate"],capture_output=True,text=True,timeout=1800)
    if out.returncode!=0:
        print("ERR",path,(out.stderr or "")[-300:],flush=True); return []
    txt=out.stdout.strip(); items=[]; dec=json.JSONDecoder(); i=0
    while i < len(txt):
        while i < len(txt) and txt[i] in " \n\r\t": i+=1
        if i>=len(txt): break
        obj,j=dec.raw_decode(txt,i); i=j
        if isinstance(obj,dict) and "items" in obj: items.extend(obj["items"])
        elif isinstance(obj,list): items.extend(obj)
        else: items.append(obj)
    return items
for r in repos:
    slug=r.replace("/","__"); allitems={}
    for a,b in windows:
        q=f"repo:{r} type:issue state:closed closed:{a}..{b}"
        path="search/issues?q="+urllib.parse.quote(q)+"&per_page=100"
        items=api(path)
        for it in items: allitems[it["number"]]=it
        print(r,a,b,len(items),"cum",len(allitems),flush=True)
        if len(items)>=1000: print("WARN CAP",r,a,b,flush=True)
    with open(os.path.join(OUT,f"list-{slug}.json"),"w") as f: json.dump(list(allitems.values()),f)
    print(r,"SAVED",len(allitems),flush=True)
print("ALL DONE",flush=True)
