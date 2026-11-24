import subprocess, json, os

os.environ["PATH"] = (
    os.path.expanduser("~/.local/share/hatch-recovery/bin") + ":" + os.environ["PATH"]
)
rows = json.load(open("/home/hatch/workspace/lane4-audit7/ledger-fleet.json"))
done = 0
for r in rows:
    res = subprocess.run(
        [
            "fleet",
            "gh",
            "api",
            f"repos/dezren39/fleet/issues/{r['number']}/labels",
            "-f",
            f"labels[]={r['label']}",
        ],
        capture_output=True,
        text=True,
        timeout=120,
    )
    ok = r["label"] in res.stdout
    done += ok
    print(
        r["number"], r["label"], "OK" if ok else "FAIL " + res.stdout[:120], flush=True
    )
print("APPLIED", done, "/", len(rows))
