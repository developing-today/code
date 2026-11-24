import subprocess, json, os, sys
os.environ["PATH"]=os.path.expanduser("~/.local/share/hatch-recovery/bin")+":"+os.environ["PATH"]
rows=json.load(open("/home/hatch/workspace/lane4-audit7/ledger-fleet.json"))
start=int(sys.argv[1]); done=0; fails=[]
for r in rows:
    if r["number"]<start: continue
    ok=False
    for attempt in range(3):
        try:
            res=subprocess.run(["fleet","gh","api",f"repos/dezren39/fleet/issues/{r['number']}/labels","-f",f"labels[]={r['label']}"],capture_output=True,text=True,timeout=180)
            ok=r["label"] in res.stdout
            if ok: break
        except Exception as e:
            print(r["number"],"EXC",e,flush=True)
    done+=ok
    if not ok: fails.append(r["number"])
    print(r["number"], r["label"], "OK" if ok else "FAIL", flush=True)
print("APPLIED", done, "FAILS", fails)
