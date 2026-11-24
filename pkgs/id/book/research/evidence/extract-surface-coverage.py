"""Inventory the pinned ID source snapshot. Uses Python's standard library only."""
from pathlib import Path
import argparse, collections, csv, json, re, tomllib
ap=argparse.ArgumentParser(description=__doc__)
ap.add_argument("source",type=Path,help="repository root at the source commit")
ap.add_argument("output",type=Path)
ap.add_argument("--commit",default="4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a")
args=ap.parse_args()
root=args.source/"pkgs/id"
rows=[]
def add(surface,name,chapter,path,lineno,source_method,facts,more=""):
    path=root/path if not Path(path).is_absolute() else Path(path)
    rel=path.relative_to(args.source)
    rows.append(dict(surface=surface,name=name,chapter=chapter,source=f"{rel}:{lineno}",source_url=f"https://github.com/developing-today/code/blob/{args.commit}/{rel}#L{lineno}",extraction=source_method,evidence=facts,deep_coverage_gap=more))
def line(path,needle):
    for ix,text in enumerate(path.read_text().splitlines(),1):
        if needle in text: return ix
    raise ValueError(f"missing source phrase {needle} in {path}")
def target(prefix):
    first=prefix.split()[1]
    if first in ("put","put-hash","get","get-hash","show","cat","list","peek","migrate-tags","id"):return "store"
    if first in ("doc","claim","writer-sets","find","search","tag"):return "graph"
    if first in ("walk","match-audit","peers","send","serve"):return "federation"
    if first in ("world",):
        tail=prefix.split()[2:]
        if tail and tail[0] in ("compile","dev","new","caps","profile","measure","doctor","install","apply","diff","verify","replay"):return "roc"
        if tail and tail[0] in ("journal","owner","fork","provenance","create","record","logs","stats"):return "lifecycle"
        if tail and tail[0] in ("mirror","replica","cross-world-write","isolation","force-cross-world-write"):return "federation"
        if tail and tail[0] in ("records","download"):return "store"
        return "clients"
    if first in ("repl",):return "clients"
    return "lifecycle"
# All Clap Subcommand enums and their actual parent variant, including Args wrappers.
scopes={"Command":"id","KeyCommand":"id key","KeysetCommand":"id keyset","RecoveryCommand":"id keyset recovery","WorldCommand":"id world","JournalCommand":"id world journal","TagCommand":"id tag","GalleryCommand":"id gallery","MatchAuditCommand":"id match-audit","WalkCommand":"id walk","GroupCommand":"id group","DocCommand":"id doc","ClaimCommand":"id claim","WriterSetCommand":"id writer-sets"}
seen=set()
for path in sorted((root/"crates/id-cli/src/cli").glob("*.rs")):
    lines=path.read_text().splitlines()
    for i,s in enumerate(lines):
        found=re.fullmatch(r"pub enum (\w+) \{",s)
        if not found or not any("derive(Subcommand" in v for v in lines[max(0,i-3):i]):continue
        enum=found.group(1);prefix=scopes[enum];seen.add(enum)
        stop=lines.index("}",i)
        for j in range(i+1,stop):
            name=re.match(r"^    ([A-Z][A-Za-z0-9]*)(?:\(|\s*\{|,)",lines[j])
            if not name:continue
            v=name.group(1)
            prev=j-1
            while prev>i and (lines[prev].startswith("    ///") or lines[prev].startswith("    #[")):prev-=1
            context=lines[prev+1:j]
            attrs=" ".join(s for s in context if s.lstrip().startswith("#["))
            explicit=re.search(r'\bname\s*=\s*"([^"]+)"',attrs)
            spelling=explicit.group(1) if explicit else re.sub(r"(?<=[a-z0-9])(?=[A-Z])","-",v).lower()
            slug=prefix+" "+spelling
            docs=[]
            for s in context:
                if not s.startswith("    ///"):continue
                text=s[7:].strip()
                if not text and docs:break
                if text:docs.append(text)
            summary=" ".join(docs)
            alias_info=[]
            for m in re.finditer(r'\balias(?:es)?\s*=\s*(\[[^\]]*\]|"[^"]+")',attrs):alias_info.extend(re.findall(r'"([^"]+)"',m.group(1)))
            cfg=re.search(r'cfg\(feature = "(\w+)"\)',attrs)
            if prefix=="id match-audit":extra="sandbox feature (parent)"
            else:extra=cfg.group(1)+" feature" if cfg else ""
            facts=summary+(" Aliases: "+", ".join(alias_info)+"." if alias_info else "")+(" Conditional: "+extra+"." if extra else "")
            add("cli",slug,target(slug),path,j+1,"Clap Subcommand enum variant, canonical kebab-case or explicit name; parent edge read from source",facts,"Arguments, command preconditions, runtime and feature matrix need chapter coverage.")
assert seen==set(scopes),seen^set(scopes)
assert all(s in [r["name"] for r in rows] for s in list(scopes.values())[1:]),"disconnected command parent"
cargo=tomllib.loads((root/"Cargo.toml").read_text())
for p in ["."]+cargo["workspace"]["members"]:
    path=root/p/"Cargo.toml";pkg=tomllib.loads(path.read_text())["package"]
    add("crate",pkg["name"],"crates",path,line(path,'name = "'+pkg["name"]+'"'),"Root Cargo.toml workspace members plus root, joined to package manifests",pkg.get("description","No package description")+". "+("Default member." if p in cargo["workspace"]["default-members"] else "Explicit package selection."),"Public types, feature combinations and caller compatibility need API-specific documentation.")
cap=root/"crates/id-caps/src/world_caps.rs"
source=cap.read_text()
for m in re.finditer(r'^    "([^"]+)" => \{\n(.*?)^    \},',source,flags=re.M|re.S):
    name,body=m.groups()
    def field(n):return re.search(r'^        '+n+r': (.*),$',body,re.M).group(1)
    add("capability",name,"roc",cap,source.count("\n",0,m.start())+1,"Production macro manifest; test probe excluded",f'{field("kind")}; args {field("argument_schema")}; ceiling {field("default_ceiling")}; scope {field("required_scope")}; manifest v{field("version")}.',"Registry membership alone cannot prove executor or per-device availability.")
for const,name in re.findall(r'pub const (CLIENT_(?:FILE_READ|NOTIFY)_NAME): &str = "([^"]+)";',source):
    add("capability",name,"roc",cap,line(cap,"pub const "+const+":"),"Client manifests in CLIENT_CATALOG_ENTRIES",f"Client-side v1 request; both world and local authority required; excluded default ceiling.","Verify the executor and real-device evidence independently.")
contract=root/"contract/shell-contract.v1.json";x=json.loads(contract.read_text())
for d in ("client","server"):
    start=line(contract,'"'+d+'_frames"');ss=contract.read_text().splitlines()
    for k,v in x[d+"_frames"].items():
        n=next(i for i,s in enumerate(ss[start:],start+1) if s.startswith('    "'+k+'":'))
        add("shell-frame",d+"."+k,"clients",contract,n,"Structured frame maps in id-shell v"+str(x["version"])+" revision "+str(x["revision"]),f'Class {v.get("class","unspecified")}; phases {", ".join(v.get("phases",[]))}; declared fields {", ".join(v.get("fields",{})) or "none"}.',"Reference the contract for optionality, authorization and cross-host replay or rendering evidence.")
route=root/"hosted-worker/src/user-do.ts"
source=route.read_text()
for m in re.finditer(r'if \(url\.pathname === "([^"]+)" && req\.method === "([^"]+)"\)',source):
    p,method=m.groups()
    add("worker-route",method+" "+p,"federation" if "/bridge/" in p else "store",route,source.count("\n",0,m.start())+1,"Literal method/path dispatch, main Worker → restricted UserDO","UserDO dispatch; authenticated upstream routing context required.","Deployed route, per-jurisdiction behavior and optional backing bindings unverified.")
template={"blobHashMatch":"/v1/blobs/{64-lowercase-hex-hash}","kvMatch":"/v1/kv/{key}"}
for m in re.finditer(r'if \((blobHashMatch|kvMatch) && req\.method === "([^"]+)"\)',source):
    key,verb=m.groups()
    add("worker-route",verb+" "+template[key],"store",route,source.count("\n",0,m.start())+1,"Regular-expression path with concrete method; validated key/hash before dispatch","Blob hash or 1–128 ASCII alphanumeric/dot/underscore/hyphen key.","Deployment and bound bucket/database evidence unverified.")
gateway=root/"hosted-worker/src/currency/gateway.ts"
routes=[("POST /v1/transfers",'if (req.method === "POST" && (path === "/v1/transfers"'),("POST /v1/holds",'if (req.method === "POST" && (path === "/v1/transfers"'),("POST /v1/holds/{id}/post","if (req.method === \"POST\" && holdAction)"),("POST /v1/holds/{id}/void","if (req.method === \"POST\" && holdAction)"),("GET /v1/balances/{account}","if (req.method === \"GET\" && balanceMatch)"),("GET /v1/transfers/{id}","if (req.method === \"GET\" && lookupMatch)")]
for k,branch in routes:
    add("currency-stub-route",k,"tokens",gateway,line(gateway,branch),"Manual expansion of literal/regex union arms in CurrencyGateway.fetch","Implemented in-process PoC gateway, outside Worker front-door mapping.","Gateway deployment, live billing and money-moving operations unverified.")
for path in sorted([p for base in ("apps","examples") for p in (root/base).rglob("*.roc")]):
    t=path.read_text();m=re.search(r"^app\s+\[program\]",t,re.M)
    if not m:continue
    name=path.relative_to(root).as_posix();negative=path.name in ["main-missing-cap.roc","main-wrong-usage.roc"]
    scope="negative Roc compile fixture" if negative else ("packaged app" if name.startswith("apps/") else "example app entry")
    add("roc-app",name,"roc",path,t.count("\n",0,m.start())+1,"Roc source app [program] header under apps/ and examples/; checked manifest path for packaged app",scope,"Binary availability and execution on each host need independent verification.")
for name in ("init","update","view","layout","records","wants","snapshot","restore"):
    p=root/"examples/roc-world/platform.roc";sig=next(s.strip().removesuffix(",") for s in p.read_text().splitlines() if s.startswith("            "+name+" :"))
    add("guest-entry",name,"roc",p,line(p,"            "+name+" :"),"Roc program requires block",sig,"Distinguish source-level requirements and optional legacy Wasm exports.")
main=root/"src/main.rs"
add("entrypoint","id (no command)","charter",main,line(main,"None => run_repl(None).await"),"Root binary default dispatch","No subcommand enters the local REPL.","Interactive REPL-only vocabulary and aliases require a separate command graph.")
facade=root/"ARCHITECTURE.md"
add("entrypoint","id Rust facade","charter",facade,line(facade,"The only crate whose"),"Architecture contract governing external Rust consumption","Root id package is the public facade for the split workspace.","List external-stability policy per API before promising that internal crates are independent public interfaces.")
for k,v in x["transports"].items():
    add("shell-transport",k,"federation",contract,line(contract,'"'+k+'": {'),"Versioned id-shell transport object",json.dumps(v,sort_keys=True),"Frame agreement alone is insufficient to show transport liveness or equivalent delivery after reconnect.")
uniqueness=[(r["surface"],r["name"]) for r in rows]
assert len(set(uniqueness))==len(rows),"nonunique row"
with (args.output/"surface-coverage.csv").open("w",newline="") as out:
    w=csv.DictWriter(out,fieldnames=rows[0].keys());w.writeheader();w.writerows(rows)
print(dict(collections.Counter(r["surface"] for r in rows)))
