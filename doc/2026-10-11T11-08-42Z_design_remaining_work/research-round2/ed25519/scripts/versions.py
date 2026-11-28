import json, sys
path = sys.argv[1]
d = json.load(open(path, encoding="utf-8"))
c = d["crate"]
print("crate:", c["id"], "| max_version:", c.get("max_version"), "| max_stable_version:", c.get("max_stable_version"), "| newest_version:", c.get("newest_version"), "| updated_at:", c.get("updated_at"), "| downloads:", c.get("downloads"), "| repo:", c.get("repository"))
print("description:", c.get("description"))
rows = d.get("versions", [])
print("total versions listed:", len(rows))
for v in rows:
    print(f'{v["num"]:<22} created={v["created_at"][:19]} yanked={v["yanked"]!s:<5} rust={v.get("rust_version")} checksum={v["checksum"][:16]} license={v.get("license")}')
