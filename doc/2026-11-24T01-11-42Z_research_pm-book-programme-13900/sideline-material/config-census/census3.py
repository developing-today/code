import os, re, pathlib, json
ROOT = pathlib.Path.home()/"git"/"code"
EXCLUDE={"node_modules",".git","dist",".wrangler"}
COMPS = [
 ("aggregator","pm-stack/aggregator"),
 ("console","pm-stack/console"),
 ("dispatch","pm-stack/dispatch"),
 ("projector","pm-stack/projector"),
 ("decision-events-worker","pm-stack/decision-events-worker"),
 ("voice-line-worker","pm-stack/voice-line-worker"),
 ("notify-relay","pm-stack/notify-relay"),
 ("pm-service","pm-stack/pm-service"),
 ("ready-for-agent","pm-stack/ready-for-agent"),
 ("platform-scripts","pm-stack/scripts"),
 ("platform-scripts","pm-stack/systemd"),
 ("platform-scripts","pm-stack/platform"),
 ("platform-scripts","pm-stack"),
 ("pkgs-pm","pkgs/pm-aggregator"),
 ("pkgs-pm","pkgs/pm-dispatch"),
 ("pkgs-pm","pkgs/pm-projector"),
 ("pkgs-pm","pkgs/pm-git-sync"),
 ("nixos-pm","nixos/services/pm-stack"),
 ("nixos-pm","nixos/services/pm-apps"),
]
# platform-scripts pm-stack root: only top-level files, handled by depth check below
def is_test(p): 
    s=str(p)
    return "/test/" in s or ".test." in s or "/tests/" in s or "test_" in p.name
rows={}  # (comp,name) -> dict(evidence, default, kind)
def add(comp,name,ev,kind,default=""):
    k=(comp,name)
    if k not in rows: rows[k]={"ev":ev,"kind":kind,"default":default}
    elif default and not rows[k]["default"]: rows[k]["default"]=default

def default_expr(line):
    m=re.search(r"(?:\?\?|\|\|)\s*(.+?)(?:,|;|\)\s*$|$)", line)
    if m:
        d=m.group(1).strip().rstrip(";,)")
        return d[:140]
    return ""

for comp, rel in COMPS:
    base=ROOT/rel
    if not base.exists(): continue
    for dp,dn,fn in os.walk(base):
        dn[:]=[d for d in dn if d not in EXCLUDE]
        for f in fn:
            p=pathlib.Path(dp)/f
            relp=str(p.relative_to(ROOT))
            if comp=="platform-scripts" and rel=="pm-stack":
                # only files directly in pm-stack/ root
                if p.parent != base: continue
            if is_test(p): continue
            if p.name in ("bun.lock","package-lock.json"): continue
            try: text=p.read_text(errors="replace")
            except: continue
            lines=text.splitlines()
            for i,line in enumerate(lines,1):
                ev=f"{relp}:{i}"
                if p.suffix in (".ts",".js",".mjs",".cjs",".tsx",".jsx",".py"):
                    for m in re.finditer(r"(?:process\.env|Bun\.env)\.([A-Z][A-Z0-9_]{1,})", line):
                        add(comp,m.group(1),ev,"process.env/Bun.env",default_expr(line))
                    for m in re.finditer(r"(?:process\.env|Bun\.env)\[['\"]([A-Z][A-Z0-9_]{1,})['\"]\]", line):
                        add(comp,m.group(1),ev,"process.env[]",default_expr(line))
                    # worker binding env.NAME only in worker src (wrangler present in component) - restrict to files with Env interface or fetch handler: use heuristic: line contains env. and file under a worker dir or src/index
                    if True:
                        for m in re.finditer(r"(?<![\w.])env\.([A-Z][A-Z0-9_]{2,})\b", line):
                            # exclude env.NODE_ENV style generic handled same
                            add(comp,m.group(1),ev,"env binding",default_expr(line))
                    for m in re.finditer(r"Deno\.env\.(?:get\(['\"])?([A-Z][A-Z0-9_]{1,})", line):
                        add(comp,m.group(1),ev,"Deno.env",default_expr(line))
                    for m in re.finditer(r"(?:os\.environ\.get\(['\"]|getenv\(['\"]|os\.environ\[['\"])([A-Z][A-Z0-9_]{1,})", line):
                        add(comp,m.group(1),ev,"python environ",default_expr(line))
                if p.suffix in (".sh",".bash") or f in ("install.sh","deploy.sh"):
                    # only ${VAR:-default} or ${VAR-default} or ${VAR:=...} forms = env-with-default evidence; plus $VAR for known PM_/QUOTA_ prefixes unassigned
                    assigned=set(re.findall(r"^([A-Z][A-Z0-9_]*)\s*=", text, re.M))|set(re.findall(r"\b([A-Z][A-Z0-9_]*)\s*=", text))
                    for m in re.finditer(r"\$\{([A-Z][A-Z0-9_]{1,})(?::[-=])([^}]*)\}", line):
                        add(comp,m.group(1),ev,"shell ${VAR:-default}",m.group(2)[:80])
                if f.endswith(".service"):
                    for m in re.finditer(r"Environment=(?:\"|')?([A-Z][A-Z0-9_]{1,})=([^\s\"']*)", line):
                        add(comp,m.group(1),ev,"systemd Environment=",m.group(2)[:100])
                if p.suffix==".nix":
                    for m in re.finditer(r"\"([A-Z][A-Z0-9_]{2,})=[^\"]*\"", line):
                        # only inside Environment lists - approximate: line mentions = with quotes in nixos files
                        add(comp,m.group(1),ev,"nixos Environment",line.strip()[:120])
                if p.name in ("compose.yml","compose.yaml","docker-compose.yml","docker-compose.yaml"):
                    for m in re.finditer(r"^\s+([A-Z][A-Z0-9_]{1,}):", line):
                        pass
                    for m in re.finditer(r"\$\{([A-Z][A-Z0-9_]{1,})(?::-([^}]*))?\}", line):
                        add(comp,m.group(1),ev,"compose ${VAR}",m.group(2) or "")
                    for m in re.finditer(r"^\s+([A-Z][A-Z0-9_]{1,}):\s*\"?([^\"\n]*)\"?\s*$", line):
                        # environment block keys: crude - only PAD_/POSTGRES_/GRIST_ known prefixes
                        if m.group(1).startswith(("PAD_","POSTGRES_","GRIST_")):
                            add(comp,m.group(1),ev,"compose environment",m.group(2)[:80])
                if p.name.startswith(".env"):
                    for m in re.finditer(r"^([A-Z][A-Z0-9_]{1,})=(.*)$", line):
                        add(comp,m.group(1),ev,".env.example",m.group(2).strip()[:100])
            # wrangler vars + bindings
            if p.name.startswith("wrangler"):
                in_vars=False
                for i,line in enumerate(lines,1):
                    ev=f"{relp}:{i}"
                    if "[vars]" in line: in_vars=True; continue
                    if in_vars:
                        m=re.match(r"\s*([A-Z][A-Z0-9_]*)\s*=\s*\"?([^\"]*)\"?", line)
                        if m: add(comp,m.group(1),ev,"wrangler [vars]",m.group(2)[:100])
                        elif line.strip().startswith("["): in_vars=False
                    m=re.search(r'"([A-Z][A-Z0-9_]{1,})"\s*:\s*"([^"]*)"', line)
                    if '"vars"' in text and m and "vars" in "\n".join(lines[max(0,i-6):i]):
                        add(comp,m.group(1),ev,"wrangler vars (jsonc)",m.group(2)[:100])
                    for m in re.finditer(r"binding\s*=\s*\"([A-Z][A-Z0-9_]+)\"|\"binding\":\s*\"([A-Z][A-Z0-9_]+)\"|bindings\s*=\s*\[\{\s*name\s*=\s*\"([A-Z][A-Z0-9_]+)\"", line):
                        n=m.group(1) or m.group(2) or m.group(3)
                        add(comp,n,ev,"wrangler binding","(binding, no scalar default)")

print("===CLEAN_TSV===")
for (comp,name) in sorted(rows):
    r=rows[(comp,name)]
    print(f"{comp}\t{name}\t{r['ev']}\t{r['kind']}\t{r['default'] if r['default'] else 'no default in code'}")
from collections import Counter
print("===COUNTS===")
print(Counter(c for c,_ in rows))
print("rows(comp,name)",len(rows),"distinct names",len(set(n for _,n in rows)))
print("\n===EXTRA_FILES===")
for rel in ["pm-stack/projector/.env.example","nixos/services/pm-stack/default.nix","pm-stack/platform/workers.json"]:
    p=ROOT/rel
    print(f"--- {rel} exists={p.exists()} ---")
    if p.exists(): print(p.read_text(errors="replace")[:9000])
