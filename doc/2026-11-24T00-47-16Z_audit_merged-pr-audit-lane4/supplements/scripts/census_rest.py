import subprocess, json, os
os.environ["PATH"]=os.path.expanduser("~/.local/share/hatch-recovery/bin")+":"+os.environ["PATH"]
def api(path):
    r=subprocess.run(["fleet","gh","api",path],capture_output=True,text=True,timeout=300)
    txt=r.stdout; return json.loads(txt[txt.find("["):])
for repo in ["developing-today/code","developing-today-forks/opencode","developing-today-forks/openchamber"]:
    out=[]; page=1
    while page<=40:
        data=api(f"repos/{repo}/pulls?state=closed&per_page=100&page={page}&sort=updated&direction=desc")
        if not data: break
        for pr in data:
            if pr.get("merged_at") and pr["merged_at"]>="2026-10-23T00:00:00Z":
                out.append({"number":pr["number"],"title":pr["title"],"merged_at":pr["merged_at"],"html_url":pr["html_url"]})
        if all((p.get("updated_at") or "")<"2026-10-23T00:00:00Z" for p in data): break
        page+=1
    fn=f"/home/hatch/workspace/lane4-audit7/census-{repo.replace('/','_')}.json"
    json.dump(out,open(fn,"w"),indent=1)
    print(repo, len(out), "pages", page, flush=True)
print("DONE")
