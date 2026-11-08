#!/usr/bin/env bash
# Apply the declarative ready-for-agent configuration (config.json,
# #1203) to the running harness, idempotently:
#
#   1. register every configured repository clone that the harness
#      does not know yet (mutation addLocalRepository),
#   2. apply the configured settings (mergePolicy, defaultModel,
#      includeAllIssueAuthors) with updateRepositorySettings — a
#      full-object input, so the current settings are read first and
#      every field the config does not name is carried over unchanged,
#   3. pause/unpause each repository to match the config.
#
# A repository whose live settings already match the config receives
# NO mutation — running this script twice is a no-op the second time.
#
#   ./apply-config.sh            apply config.json next to this script
#   DRY_RUN=1 ./apply-config.sh  print the mutations it would send
#
# Overrides: RFA_CONFIG (config path), RFA_GRAPHQL_URL (endpoint),
# RFA_REPOS_DIR (clone root, default ./repos next to the config).
set -euo pipefail

cd "$(dirname "$0")"
CONFIG="${RFA_CONFIG:-$PWD/config.json}"
GRAPHQL_URL="${RFA_GRAPHQL_URL:-}"
REPOS_DIR="${RFA_REPOS_DIR:-$PWD/repos}"
export CONFIG GRAPHQL_URL REPOS_DIR
export DRY_RUN="${DRY_RUN:-}"

python3 - <<'PYEOF'
import json, os, sys, urllib.request

config_path = os.environ["CONFIG"]
with open(config_path) as f:
    config = json.load(f)
url = os.environ.get("GRAPHQL_URL") or config.get("graphqlUrl") or "http://127.0.0.1:6056/graphql"
repos_dir = os.environ["REPOS_DIR"]
dry_run = bool(os.environ.get("DRY_RUN"))

def gql(query, variables=None):
    body = {"query": query}
    if variables is not None:
        body["variables"] = variables
    req = urllib.request.Request(
        url,
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=15) as res:
        payload = json.load(res)
    if payload.get("errors"):
        raise SystemExit(f"GraphQL error: {json.dumps(payload['errors'])[:400]}")
    return payload["data"]

REPO_FIELDS = (
    "id projectPath localPath paused mergePolicy defaultModel "
    "selectedAgentBackend defaultThinkingLevel reviewModel "
    "reviewThinkingLevel includeAllIssueAuthors "
    "waitForReadyForReviewChecks automaticCiRepair forge forgeHost "
    "issueTracker linearProjectId linearProjectName"
)

def list_repositories():
    return gql("{ repositories { %s } }" % REPO_FIELDS)["repositories"]

# Config-owned fields: config key -> settings-input key.
OWNED = [
    ("mergePolicy", "mergePolicy"),
    ("defaultModel", "defaultModel"),
    ("includeAllIssueAuthors", "includeAllIssueAuthors"),
    ("selectedAgentBackend", "selectedAgentBackend"),
]

by_path = {r["projectPath"]: r for r in list_repositories()}

for entry in config["repositories"]:
    project = entry["projectPath"]
    repo = by_path.get(project)
    if repo is None:
        local_path = os.path.join(repos_dir, entry["localDir"])
        if dry_run:
            print(f"DRY-RUN would register {project}: addLocalRepository(path={local_path})")
            continue
        if not os.path.isdir(local_path):
            print(
                f"skip {project}: clone missing at {local_path} "
                "(run ./install.sh clone first)",
                file=sys.stderr,
            )
            continue
        gql(
            "mutation($path: String!) { addLocalRepository(path: $path) { id } }",
            {"path": local_path},
        )
        by_path = {r["projectPath"]: r for r in list_repositories()}
        repo = by_path.get(project)
        if repo is None:
            raise SystemExit(f"addLocalRepository did not register {project}")
        print(f"registered {project} ({repo['id']})")

    # Settings: full-object input built from the live settings, with
    # only the config-owned fields overridden.
    settings = {
        "repositoryId": repo["id"],
        "forge": repo.get("forge"),
        "forgeHost": repo.get("forgeHost"),
        "projectPath": repo.get("projectPath"),
        "paused": repo.get("paused"),
        "selectedAgentBackend": repo.get("selectedAgentBackend"),
        "defaultModel": repo.get("defaultModel"),
        "defaultThinkingLevel": repo.get("defaultThinkingLevel"),
        "reviewModel": repo.get("reviewModel"),
        "reviewThinkingLevel": repo.get("reviewThinkingLevel"),
        "mergePolicy": repo.get("mergePolicy"),
        "includeAllIssueAuthors": repo.get("includeAllIssueAuthors"),
        "waitForReadyForReviewChecks": repo.get("waitForReadyForReviewChecks"),
        "automaticCiRepair": repo.get("automaticCiRepair"),
        "issueTracker": repo.get("issueTracker"),
        "linearProjectId": repo.get("linearProjectId"),
        "linearProjectName": repo.get("linearProjectName"),
    }
    drift = []
    for config_key, input_key in OWNED:
        if config_key in entry and entry[config_key] != settings[input_key]:
            settings[input_key] = entry[config_key]
            drift.append(input_key)
    if drift:
        if dry_run:
            print(f"DRY-RUN would update {project} settings: {', '.join(drift)}")
        else:
            gql(
                "mutation($input: UpdateRepositorySettingsInput!) "
                "{ updateRepositorySettings(input: $input) { id } }",
                {"input": settings},
            )
            print(f"updated {project} settings: {', '.join(drift)}")
    else:
        print(f"{project} settings already match config")

    want_paused = bool(entry.get("paused", False))
    if want_paused != bool(repo.get("paused")):
        mutation = "pauseRepository" if want_paused else "unpauseRepository"
        if dry_run:
            print(f"DRY-RUN would {mutation} {project}")
        else:
            gql(
                "mutation($id: ID!) { %s(repositoryId: $id) { id } }" % mutation,
                {"id": repo["id"]},
            )
            print(f"{mutation} applied to {project}")

print("ready-for-agent config applied" + (" (dry run)" if dry_run else ""))
PYEOF
