import subprocess, json, os
os.environ["PATH"]=os.path.expanduser("~/.local/share/hatch-recovery/bin")+":"+os.environ["PATH"]
def api(path):
    r=subprocess.run(["fleet","gh","api",path],capture_output=True,text=True,timeout=300)
    txt=r.stdout; idx=[x for x in [txt.find("["),txt.find("{")] if x>=0]
    return json.loads(txt[min(idx):])
targets=[("dezren39/fleet",n) for n in [107,89,88,32,30,70,28,46,87]]+[("developing-today/code",n) for n in [13774,13189,13537,2230,6645]]
out={}
for repo,n in targets:
    try:
        iss=api(f"repos/{repo}/issues/{n}")
        com=api(f"repos/{repo}/issues/{n}/comments?per_page=100")
        out[f"{repo}#{n}"]={"title":iss["title"],"state":iss["state"],"labels":[l["name"] for l in iss["labels"]],"body":iss.get("body") or "","comments":[{"user":c["user"]["login"],"created":c["created_at"],"body":c["body"]} for c in com]}
        print("ok",repo,n,iss["state"],len(com),flush=True)
    except Exception as e:
        out[f"{repo}#{n}"]={"error":str(e)}; print("ERR",repo,n,e,flush=True)
json.dump(out,open("/home/hatch/workspace/lane4-audit7/fleet-issues-full.json","w"),indent=1)
print("DONE")
