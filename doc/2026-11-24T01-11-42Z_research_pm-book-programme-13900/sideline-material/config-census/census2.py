import os, re, pathlib
ROOT = pathlib.Path.home()/"git"/"code"
EXCLUDE={"node_modules",".git","dist",".wrangler"}
def walk(rel):
    base=ROOT/rel
    if not base.exists(): return
    for dp,dn,fn in os.walk(base):
        dn[:]=[d for d in dn if d not in EXCLUDE]
        for f in fn:
            yield pathlib.Path(dp)/f

print("===WORKER_DIRS===")
for rel in ["pm-stack/decision-events-worker","pm-stack/voice-line-worker","pm-stack/notify-relay","pm-stack/console","pm-stack/dispatch","pm-stack/projector","pm-stack/platform","pm-stack/shared-schema"]:
    base=ROOT/rel
    print(rel, "exists", base.exists())
    if base.exists():
        for p in sorted(walk(rel)):
            print(" ", str(p.relative_to(ROOT)), p.stat().st_size)

print("\n===WRANGLER_CONTENTS===")
for rel in ["pm-stack","pkgs/pm-aggregator","pkgs/pm-dispatch","pkgs/pm-projector","pkgs/pm-git-sync"]:
    for p in walk(rel) or []:
        if p.name.startswith("wrangler"):
            print(f"--- {p.relative_to(ROOT)} ---")
            print(p.read_text(errors="replace")[:6000])

print("\n===ENV_INTERFACES_AND_DESTRUCTURING===")
for rel in ["pm-stack"]:
    for p in walk(rel):
        if p.suffix not in (".ts",".js",".mjs",".tsx"): continue
        if "bun.lock" in p.name: continue
        text=p.read_text(errors="replace")
        for i,line in enumerate(text.splitlines(),1):
            if re.search(r"(interface\s+Env\b|type\s+Env\s*=|Bindings\s*:)", line) or re.search(r"\{\s*[A-Z_][A-Z0-9_,\s]*\}\s*=\s*(process\.env|Bun\.env|env\b)", line) or "import.meta.env" in line or re.search(r"c\.env\.", line) or re.search(r"\benv\[", line):
                print(f"{p.relative_to(ROOT)}:{i}: {line.strip()[:300]}")
            # Env interface block members handled by printing following lines if interface Env
        # print Env interface blocks fully
        m=re.search(r"interface Env\s*\{([^}]*)\}", text, re.S)
        if m:
            print(f"ENVIFACE {p.relative_to(ROOT)}: {m.group(1).strip()[:800]}")

print("\n===SYSTEMD_ENV_LINES===")
for rel in ["pm-stack","nixos/services/pm-stack","nixos/services/pm-apps"]:
    for p in walk(rel):
        if p.suffix==".service" or p.suffix==".nix":
            for i,line in enumerate(p.read_text(errors="replace").splitlines(),1):
                if "Environment" in line or "environment." in line:
                    print(f"{p.relative_to(ROOT)}:{i}: {line.strip()[:300]}")

print("\n===COMPOSE_ENV===")
for rel in ["pm-stack"]:
    for p in walk(rel):
        if p.name in ("compose.yml","compose.yaml","docker-compose.yml","docker-compose.yaml"):
            print(f"--- {p.relative_to(ROOT)} ---")
            print(p.read_text(errors="replace")[:6000])
