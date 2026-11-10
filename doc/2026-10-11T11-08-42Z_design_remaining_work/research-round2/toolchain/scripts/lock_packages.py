import json
import sys
import tomllib

# Read a Cargo.lock (TOML) given as the first argument; write its package list as JSON to the second argument.
lock_path, out_path = sys.argv[1], sys.argv[2]
with open(lock_path, "rb") as f:
    lock = tomllib.load(f)
pkgs = []
for p in lock.get("package", []):
    src = p.get("source", "path+local")
    pkgs.append({"name": p["name"], "version": p["version"], "source": src})
with open(out_path, "w") as f:
    json.dump(pkgs, f, indent=1)
registry = [p for p in pkgs if p["source"].startswith("registry+")]
git = [p for p in pkgs if p["source"].startswith("git+")]
print(f"lock={lock_path} lockfile-version={lock.get('version')} packages={len(pkgs)} registry={len(registry)} git={len(git)}")
KEY = ["iroh", "iroh-relay", "iroh-blobs", "iroh-tickets", "iroh-docs", "iroh-gossip", "redb", "cbor-core", "dcbor", "beekem",
       "turso", "turso_core", "libsql", "ed25519-dalek", "p2panda-core", "p2panda-auth", "p2panda-encryption",
       "p2panda-sync", "p2panda-store", "p2panda-stream", "tokio", "distributed-topic-tracker"]
for p in sorted(pkgs, key=lambda x: (x["name"], x["version"])):
    if p["name"] in KEY:
        print(f"  {p['name']} {p['version']}  {p['source'][:90]}")
