import sys
import tomllib

# For a channel manifest path (argv[1]), print availability of the repo's components on x86_64-unknown-linux-gnu.
with open(sys.argv[1], "rb") as f:
    m = tomllib.load(f)
pkgs = m.get("pkg", {})
for name in ("rustfmt", "clippy-preview", "rust-analyzer", "llvm-tools-preview", "rustc", "cargo"):
    p = pkgs.get(name, {})
    avail = p.get("target", {}).get("x86_64-unknown-linux-gnu", {}).get("available")
    print(f"{name}: version={p.get('version')!r} x86_64-unknown-linux-gnu.available={avail}")
