import subprocess, json, os
os.environ["PATH"]=os.path.expanduser("~/.local/share/hatch-recovery/bin")+":"+os.environ["PATH"]
def api(path):
    r=subprocess.run(["fleet","gh","api",path],capture_output=True,text=True,timeout=300)
    txt=r.stdout; return json.loads(txt[txt.find("["):])
out=[]; page=1
while True:
    data=api(f"repos/dezren39/fleet/pulls?state=closed&per_page=100&page={page}&sort=updated&direction=desc")
    if not data: break
    for pr in data:
        if pr.get("merged_at") and pr["merged_at"]>="2026-10-23T00:00:00Z":
            out.append({"number":pr["number"],"title":pr["title"],"merged_at":pr["merged_at"],"html_url":pr["html_url"]})
    if all((p.get("updated_at") or "")<"2026-10-23T00:00:00Z" for p in data): break
    page+=1
json.dump(out,open(os.path.expanduser("~/workspace/lane4-audit7/census-fleet.json"),"w"),indent=1)
print(len(out))
