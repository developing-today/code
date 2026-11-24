import os, re, json, pathlib
ROOT = pathlib.Path.home()/"git"/"code"
SCOPES = {
 "aggregator": ["pm-stack/aggregator"],
 "console": ["pm-stack/console"],
 "dispatch": ["pm-stack/dispatch"],
 "projector": ["pm-stack/projector"],
 "decision-events-worker": ["pm-stack/decision-events-worker"],
 "voice-line-worker": ["pm-stack/voice-line-worker"],
 "notify-relay": ["pm-stack/notify-relay"],
 "pm-service": ["pm-stack/pm-service"],
 "ready-for-agent": ["pm-stack/ready-for-agent"],
 "platform-scripts": ["pm-stack/scripts","pm-stack/systemd","pm-stack/scheduled-history"],
 "pkgs": ["pkgs/pm-aggregator","pkgs/pm-dispatch","pkgs/pm-projector","pkgs/pm-git-sync"],
 "nixos-pm": ["nixos/services/pm-stack","nixos/services/pm-apps"],
}
# also discover top-level pm-stack dirs
pmstack = ROOT/"pm-stack"
print("PMSTACK_DIRS:", sorted([p.name for p in pmstack.iterdir() if p.is_dir()]))
for k,v in list(SCOPES.items()):
    for rel in v:
        print(f"SCOPE {k} {rel} exists={(ROOT/rel).exists()}")

EXCLUDE_DIRS={"node_modules",".git","dist",".wrangler"}
CODE_EXTS={".ts",".js",".mjs",".cjs",".tsx",".jsx",".py",".sh",".bash",".nix",".toml",".json",".jsonc",".yml",".yaml",".service",".md"}
# patterns
pats = [
 ("process.env", re.compile(r"process\.env\.([A-Z][A-Z0-9_]{1,})")),
 ("process.env[]", re.compile(r"process\.env\[['\"]([A-Z][A-Z0-9_]{1,})['\"]\]")),
 ("Bun.env", re.compile(r"Bun\.env\.([A-Z][A-Z0-9_]{1,})")),
 ("env-binding", re.compile(r"(?<![\w.])env\.([A-Z][A-Z0-9_]{2,})\b")),
 ("Deno.env", re.compile(r"Deno\.env\.(?:get\(['\"])?([A-Z][A-Z0-9_]{1,})")),
 ("getenv-py", re.compile(r"(?:os\.environ(?:\.get)?\(?\[?['\"]|getenv\(['\"])([A-Z][A-Z0-9_]{1,})")),
 ("shell-var", re.compile(r"\$\{([A-Z][A-Z0-9_]{1,})(?::[-=?][^}]*)?\}")),
 ("shell-var2", re.compile(r"(?<![\w])\$([A-Z][A-Z0-9_]{2,})\b")),
 ("systemd-env", re.compile(r"Environment=(?:\"|')?([A-Z][A-Z0-9_]{1,})=")),
 ("compose-env", re.compile(r"^\s+-?\s*([A-Z][A-Z0-9_]{2,})=", re.M)),
]
# default extraction: look on same line for || 'x' or ?? 'x' or or "x" or default
def default_on_line(line):
    m = re.search(r"(?:\|\||\?\?)\s*['\"]([^'\"]*)['\"]", line)
    if m: return m.group(1)
    m = re.search(r"(?:\|\||\?\?)\s*(\d+)", line)
    if m: return m.group(1)
    m = re.search(r"fallback|default", line, re.I)
    return None

results = {}  # (component, name) -> list of (path:line, pattern, line_snippet, default)
all_files=[]
for comp, rels in SCOPES.items():
    for rel in rels:
        base = ROOT/rel
        if not base.exists(): continue
        for dirpath, dirnames, filenames in os.walk(base):
            dirnames[:] = [d for d in dirnames if d not in EXCLUDE_DIRS]
            for fn in filenames:
                fp = pathlib.Path(dirpath)/fn
                relp = str(fp.relative_to(ROOT))
                all_files.append((comp, relp))
                if fp.suffix not in CODE_EXTS and fn not in ("wrangler.toml",) and not fn.endswith(".service"):
                    # still scan .env.example etc
                    if not (fn.startswith(".env") or fn in ("Dockerfile",)):
                        continue
                try:
                    text = fp.read_text(errors="replace")
                except: continue
                # skip huge lockfiles
                if fn in ("bun.lock","package-lock.json"): continue
                for i, line in enumerate(text.splitlines(),1):
                    if len(line)>2000: continue
                    for pname, rx in pats:
                        # shell patterns only in shell-ish files
                        if pname.startswith("shell") and fp.suffix not in (".sh",".bash",".service",".yml",".yaml") and fn not in ("install.sh",):
                            # also allow in .ts? no
                            if fp.suffix in (".ts",".js"): continue
                        if pname=="compose-env" and fp.suffix not in (".yml",".yaml",".nix",".service",".toml"): continue
                        if pname=="systemd-env" and not fn.endswith(".service") and fp.suffix!=".nix": continue
                        if pname=="env-binding" and fp.suffix not in (".ts",".js",".mjs"): continue
                        for m in rx.finditer(line):
                            name=m.group(1)
                            # filter noise
                            if name in ("PATH","HOME","NODE_ENV"): 
                                pass # keep NODE_ENV? keep all but note
                            key=(comp,name)
                            results.setdefault(key, []).append((f"{relp}:{i}", pname, line.strip()[:300], default_on_line(line)))

# Print TSV sorted
print("\n===CENSUS_TSV===")
for (comp,name) in sorted(results.keys()):
    ev = results[(comp,name)]
    first = ev[0]
    defaults = sorted(set(e[3] for e in ev if e[3]))
    print(f"{comp}\t{name}\t{first[0]}\t{first[1]}\t{';'.join(defaults) if defaults else ''}\tcount={len(ev)}")
print("\n===ALL_EVIDENCE===")
for (comp,name) in sorted(results.keys()):
    for (pathline, pname, snippet, dflt) in results[(comp,name)]:
        print(f"{comp}\t{name}\t{pathline}\t{pname}\tdefault={dflt or ''}\t{snippet}")
print("\n===CONFIG_FILES===")
for comp, relp in sorted(set(all_files)):
    low=relp.lower()
    if any(x in low for x in ["wrangler","compose","docker-compose",".service",".env","bunfig","tsconfig","package.json"]) or relp.endswith((".toml",)):
        print(f"{comp}\t{relp}")
print("\n===FILE_COUNTS===")
from collections import Counter
print(Counter(c for c,_ in all_files))
print("TOTAL distinct (comp,name):", len(results))
print("TOTAL distinct names:", len(set(n for _,n in results)))
# docs-only: search doc/pm-stack + pm-stack READMEs/docs for PM_ / env names not in code set
code_names=set(n for _,n in results)
print("\n===DOCS_SCAN===")
doc_roots=[ROOT/"doc/pm-stack", ROOT/"pm-stack"]
doc_names={}
for base in doc_roots:
    if not base.exists(): continue
    for dirpath, dirnames, filenames in os.walk(base):
        dirnames[:]=[d for d in dirnames if d not in EXCLUDE_DIRS]
        for fn in filenames:
            if not fn.endswith((".md",".mdx",".txt")): continue
            fp=pathlib.Path(dirpath)/fn
            relp=str(fp.relative_to(ROOT))
            try: text=fp.read_text(errors="replace")
            except: continue
            for i,line in enumerate(text.splitlines(),1):
                for m in re.finditer(r"\b([A-Z][A-Z0-9_]{3,})\b", line):
                    n=m.group(1)
                    # only env-like: contains _ and (PM_ or _URL/_KEY/_TOKEN/_SECRET/_HOST/_PORT/_DIR/_PATH/_ID)
                    if "_" in n and (n.startswith("PM_") or any(s in n for s in ["_URL","_TOKEN","_SECRET","_KEY"])):
                        doc_names.setdefault(n, []).append(f"{relp}:{i}")
for n in sorted(doc_names):
    tag="IN_CODE" if n in code_names else "DOCS_ONLY"
    print(f"{tag}\t{n}\t{doc_names[n][0]}\tcount={len(doc_names[n])}")
