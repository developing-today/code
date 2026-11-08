#!/usr/bin/env bash
# Stubbed-GraphQL test for apply-config.sh (#1203): an in-memory fake
# harness records every mutation; the first apply must register +
# configure the repo, the second apply must send NO mutations
# (idempotence), and DRY_RUN must send nothing at all.
set -euo pipefail

cd "$(dirname "$0")"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"; kill "${STUB_PID:-}" 2>/dev/null || true' EXIT

mkdir -p "$WORK/repos/code"
cat > "$WORK/config.json" <<'JSON'
{
  "version": 1,
  "graphqlUrl": "http://127.0.0.1:16056/graphql",
  "repositories": [
    {
      "projectPath": "developing-today/code",
      "cloneUrl": "https://github.com/developing-today/code.git",
      "localDir": "code",
      "mergePolicy": "OFF",
      "defaultModel": "opencode-go/kimi-k3.1-pro",
      "includeAllIssueAuthors": true,
      "paused": false
    }
  ]
}
JSON

STUB_LOG="$WORK/mutations.log" STUB_PORT=16056 python3 - <<'PYEOF' &
import json, os
from http.server import BaseHTTPRequestHandler, HTTPServer

LOG = os.environ["STUB_LOG"]
state = {"repositories": []}

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        query = body.get("query", "")
        variables = body.get("variables") or {}
        data = {}
        if "addLocalRepository" in query:
            with open(LOG, "a") as f:
                f.write("addLocalRepository\n")
            state["repositories"] = [{
                "id": "repo-stub-1",
                "projectPath": "developing-today/code",
                "localPath": variables["path"],
                "paused": True,
                "mergePolicy": None,
                "defaultModel": None,
                "selectedAgentBackend": None,
                "defaultThinkingLevel": None,
                "reviewModel": None,
                "reviewThinkingLevel": None,
                "includeAllIssueAuthors": False,
                "waitForReadyForReviewChecks": None,
                "automaticCiRepair": None,
                "forge": "github",
                "forgeHost": "github.com",
                "issueTracker": "github",
                "linearProjectId": None,
                "linearProjectName": None,
            }]
            data = {"addLocalRepository": {"id": "repo-stub-1"}}
        elif "updateRepositorySettings" in query:
            with open(LOG, "a") as f:
                f.write("updateRepositorySettings\n")
            incoming = variables["input"]
            repo = state["repositories"][0]
            for key, value in incoming.items():
                if key != "repositoryId":
                    repo[key] = value
            data = {"updateRepositorySettings": {"id": repo["id"]}}
        elif "unpauseRepository" in query or "pauseRepository" in query:
            name = "unpauseRepository" if "unpauseRepository" in query else "pauseRepository"
            with open(LOG, "a") as f:
                f.write(name + "\n")
            state["repositories"][0]["paused"] = name == "pauseRepository"
            data = {name: {"id": "repo-stub-1"}}
        elif "repositories" in query:
            data = {"repositories": state["repositories"]}
        payload = json.dumps({"data": data}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

HTTPServer(("127.0.0.1", int(os.environ["STUB_PORT"])), Handler).serve_forever()
PYEOF
STUB_PID=$!
sleep 1

export RFA_CONFIG="$WORK/config.json"
export RFA_REPOS_DIR="$WORK/repos"
export RFA_GRAPHQL_URL="http://127.0.0.1:16056/graphql"

: > "$WORK/mutations.log"
DRY_RUN=1 ./apply-config.sh > "$WORK/dry-run.out"
grep -q "would register" "$WORK/dry-run.out"
[[ ! -s "$WORK/mutations.log" ]] || { echo "FAIL: dry run mutated the harness"; exit 1; }
echo "ok: dry run plans registration and mutates nothing"

./apply-config.sh > "$WORK/first.out"
grep -q "addLocalRepository" "$WORK/mutations.log"
grep -q "updateRepositorySettings" "$WORK/mutations.log"
grep -q "unpauseRepository" "$WORK/mutations.log"
echo "ok: first apply registers, configures, unpauses"

: > "$WORK/mutations.log"
./apply-config.sh > "$WORK/second.out"
grep -q "already match config" "$WORK/second.out"
[[ ! -s "$WORK/mutations.log" ]] || { echo "FAIL: second apply was not a no-op:"; cat "$WORK/mutations.log"; exit 1; }
echo "ok: second apply is a no-op (idempotent)"

echo "apply-config.test: PASS"
