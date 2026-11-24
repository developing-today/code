import subprocess, json, os, sys
os.environ["PATH"]=os.path.expanduser("~/.local/share/hatch-recovery/bin")+":"+os.environ["PATH"]
def api(path):
    r=subprocess.run(["fleet","gh","api",path],capture_output=True,text=True,timeout=300)
    txt=r.stdout; idx=[x for x in [txt.find("["),txt.find("{")] if x>=0]
    if not idx: raise RuntimeError(path+" NOJSON "+txt[:200])
    return json.loads(txt[min(idx):])
repo="dezren39/fleet"; census=json.load(open("/home/hatch/workspace/lane4-audit7/census-fleet.json")); outfn="/home/hatch/workspace/lane4-audit7/full-dezren39_fleet.json"
out=[]
for i,entry in enumerate(census):
    n=entry["number"]
    try:
        pr=api(f"repos/{repo}/pulls/{n}")
        files=api(f"repos/{repo}/pulls/{n}/files?per_page=100")
        out.append({"number":n,"title":pr["title"],"merged_at":pr.get("merged_at"),"user":pr["user"]["login"],"body":pr.get("body") or "","additions":pr.get("additions"),"deletions":pr.get("deletions"),"changed_files":pr.get("changed_files"),"base":pr["base"]["ref"],"head":pr["head"]["ref"],"files":[{"filename":f["filename"],"status":f["status"],"additions":f["additions"],"deletions":f["deletions"],"patch":(f.get("patch") or "")[:6000]} for f in files]})
        print("ok",n,i+1,"/",len(census),flush=True)
    except Exception as e:
        out.append({"number":n,"error":str(e)}); print("ERR",n,e,flush=True)
    json.dump(out,open(outfn,"w"),indent=1)
print("DONE",len(out))
