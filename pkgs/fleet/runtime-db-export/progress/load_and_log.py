#!/usr/bin/env python3
import json, shutil, subprocess, sys, os
HERE = os.path.expanduser("~/tools/runtime-db-export")
src, page = sys.argv[1], sys.argv[2]
size = sys.argv[3] if len(sys.argv) > 3 else "100"
keep = os.path.join(HERE, "progress", "cur_page.json")
shutil.copyfile(src, keep)
p = subprocess.run([sys.executable, os.path.join(HERE, "page_load.py"), keep, "spec_m3.json", "updated_at,agent_id"], cwd=HERE, capture_output=True, text=True)
sys.stdout.write(p.stdout); sys.stderr.write(p.stderr)
if p.returncode != 0:
    sys.exit(p.returncode)
d = json.loads(p.stdout.strip().splitlines()[-1])
last = d["last"]
line = f"page {page} rows={d['count']} cursor=({last.get('updated_at')},{last.get('agent_id')}) size={size}\n"
with open(os.path.join(HERE, "progress", "m3.log"), "a") as f:
    f.write(line)
print("logged:", line.strip())
