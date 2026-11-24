import json, glob, subprocess, os
paths=sorted(p for p in glob.glob("/home/hatch/workspace/lane4-audit7/packets/*.json") if not p.endswith("manifest.json"))
results=[]
for path in paths:
    pkt=json.load(open(path)); sl=pkt["payload"]["slice"]
    pkt["dedupe_key"]=f"repo-health:v1:slice:{sl['repo']}:issue:13879:slice-{sl['batch']}"
    json.dump(pkt,open(path,"w"),indent=1)
    script="cat > /tmp/audit7-pkt.json <<'PKTEOF'\n"+json.dumps(pkt)+"\nPKTEOF\nfleet queue enqueue /tmp/audit7-pkt.json --queue repo-health-code-pilot 2>&1 | tail -4\n"
    open("/tmp/l4enq.sh","w").write(script)
    r=subprocess.run(["bash","-c","timeout 110 ~/tools/laptop-ssh.sh 'bash -s' < /tmp/l4enq.sh"],capture_output=True,text=True)
    out=(r.stdout+r.stderr).strip().replace("\n"," ")[-260:]
    print(os.path.basename(path),"->",out,flush=True)
    results.append({"packet":os.path.basename(path),"dedupe_key":pkt["dedupe_key"],"result":out})
json.dump(results,open("/home/hatch/workspace/lane4-audit7/enqueue-results.json","w"),indent=1)
print("DONE",len(results))
