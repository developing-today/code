import sys
import tomllib

# Read a Rust channel manifest (TOML) given as a path argument and print the stable facts.
path = sys.argv[1]
with open(path, "rb") as f:
    m = tomllib.load(f)
print("manifest-version:", m.get("manifest-version"))
print("date:", m.get("date"))
pkgs = m.get("pkg", {})
for name in ("rustc", "cargo", "clippy-preview", "rustfmt-preview", "rust-std", "rust-src"):
    p = pkgs.get(name)
    if not p:
        print(name, "MISSING")
        continue
    line = f"{name}: version={p.get('version')!r} git_commit_hash={p.get('git_commit_hash')!r}"
    if name in ("rustc", "cargo"):
        avail = p.get("target", {}).get("x86_64-unknown-linux-gnu", {}).get("available")
        line += f" x86_64-unknown-linux-gnu.available={avail}"
    print(line)
