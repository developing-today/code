import os, sys, subprocess
# Reads the Cloudflare API token from stdin (never from disk/args),
# exports it transiently for the child process, runs wrangler with given args.
token = sys.stdin.read().strip()
if not token:
    print("no token received on stdin", file=sys.stderr)
    sys.exit(2)
env = dict(os.environ)
env["CLOUDFLARE_API_TOKEN"] = token
r = subprocess.run(["/home/hatch/tools/bin/wrangler"] + sys.argv[1:], env=env,
                   timeout=300)
sys.exit(r.returncode)
