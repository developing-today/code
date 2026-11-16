import os, sys, json, subprocess
# Reads {"token":..., "name":..., "value":...} from stdin.
# Sets CLOUDFLARE_API_TOKEN transiently and pipes value to `wrangler secret put`.
cfg = json.load(sys.stdin)
env = dict(os.environ)
env["CLOUDFLARE_API_TOKEN"] = cfg["token"]
r = subprocess.run(
    ["/home/hatch/tools/bin/wrangler", "secret", "put", cfg["name"]],
    # NO trailing newline: wrangler stores stdin bytes verbatim, and a stray
    # "\n" breaks timingSafeEqual checks and SigV4 signatures (hit 2026-10-07).
    env=env, input=cfg["value"], capture_output=True, text=True,
    timeout=180, cwd="/home/hatch/git/nix-cache")
print(r.stdout[-400:])
print(r.stderr[-400:], file=sys.stderr)
sys.exit(r.returncode)
