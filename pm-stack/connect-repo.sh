#!/usr/bin/env bash
# Connect a GitHub repository to its Kaneo project and import its open
# issues, using the project registry in kaneo-projects.json.
#
# Usage:
#   ./connect-repo.sh status          per project: does the developing.today
#                                     app installation cover the repo, is the
#                                     Kaneo integration connected, task and
#                                     open-issue counts
#   ./connect-repo.sh connect <slug>  verify -> connect -> import (resumes
#                                     until done) -> print both counts
#
# Installing the app on a repository is NOT done here: adding a
# repository to an installation needs an org owner's (or, for a
# personal account, the account owner's) GitHub session — the app's
# own credentials cannot do it. Install first, then run connect.
#
# Needs: sops (+ the age key), openssl, curl, python3, docker compose
# (task counts are read from Kaneo's database). Secrets are read from
# ../secrets/pm-stack.sops.yaml and never printed.
set -euo pipefail

cd "$(dirname "$0")"
STACK_DIR="$PWD"
SECRETS_FILE="${PM_STACK_SECRETS:-$STACK_DIR/../secrets/pm-stack.sops.yaml}"
export SOPS_AGE_KEY_FILE="${SOPS_AGE_KEY_FILE:-$HOME/.config/sops/age/keys.txt}"
REGISTRY="$STACK_DIR/kaneo-projects.json"
GH_API=https://api.github.com

WORK="$(mktemp -d)"
chmod 700 "$WORK"
trap 'rm -rf "$WORK"' EXIT

secret() { sops --decrypt --extract "[\"$1\"]" "$SECRETS_FILE"; }
reg() { python3 -c "import json,sys;r=json.load(open('$REGISTRY'));$1" "${@:2}"; }
json() { python3 -c "import json,sys;d=json.load(sys.stdin);$1"; }

KANEO_URL="$(reg 'print(r["kaneoUrl"])')"

app_jwt() {
  secret github_private_key_base64 | base64 -d >"$WORK/app.pem"
  local b64 now h p s
  b64() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }
  now=$(date +%s)
  h=$(printf '{"alg":"RS256","typ":"JWT"}' | b64)
  p=$(printf '{"iat":%d,"exp":%d,"iss":"%s"}' $((now - 60)) $((now + 540)) "$(secret github_app_id)" | b64)
  s=$(printf '%s.%s' "$h" "$p" | openssl dgst -sha256 -sign "$WORK/app.pem" | b64)
  rm -f "$WORK/app.pem"
  printf '%s.%s.%s' "$h" "$p" "$s"
}

# Installation id covering owner/repo, or empty when the app is not installed there.
repo_installation() {
  curl -s -H "Authorization: Bearer $JWT" -H 'Accept: application/vnd.github+json' \
    "$GH_API/repos/$1/installation" </dev/null | json 'print(d.get("id",""))'
}

installation_token() {
  curl -s -X POST -H "Authorization: Bearer $JWT" -H 'Accept: application/vnd.github+json' \
    "$GH_API/app/installations/$1/access_tokens" </dev/null | json 'print(d["token"])'
}

# Open issues (pull requests excluded) — what Kaneo's import brings across.
open_issue_count() {
  curl -s -G -H "Authorization: token $2" -H 'Accept: application/vnd.github+json' \
    --data-urlencode "q=repo:$1 is:issue is:open" "$GH_API/search/issues?per_page=1" </dev/null |
    json 'print(d["total_count"])'
}

psql_value() {
  docker compose -f "$STACK_DIR/compose.yml" exec -T postgres \
    psql -U kaneo kaneo -tAc "$1"
}

kaneo_counts() {
  psql_value "select
    (select count(*) from task where project_id = '$1') || ' tasks, ' ||
    (select count(*) from external_link l join integration i on i.id = l.integration_id
       where i.project_id = '$1' and l.resource_type = 'issue') || ' issue links, ' ||
    (select count(*) from task t join \"column\" c on c.id = t.column_id
       where t.project_id = '$1' and c.slug <> 'done') || ' not in Done'"
}

kaneo_login() {
  secret kaneo_admin_email >"$WORK/email"
  secret kaneo_admin_password >"$WORK/password"
  python3 -c 'import json,sys;print(json.dumps({"email":open(sys.argv[1]).read(),"password":open(sys.argv[2]).read()}))' \
    "$WORK/email" "$WORK/password" >"$WORK/signin.json"
  rm -f "$WORK/email" "$WORK/password"
  curl -sf -c "$WORK/cookies" -H 'Content-Type: application/json' \
    -H 'Origin: https://kaneo.security.cab' -d @"$WORK/signin.json" \
    "$KANEO_URL/api/auth/sign-in/email" >/dev/null
  rm -f "$WORK/signin.json"
}

kaneo() { curl -s -b "$WORK/cookies" -H 'Content-Type: application/json' "$@"; }

cmd_status() {
  JWT="$(app_jwt)"
  local slug project repo inst token
  for slug in $(reg 'print(" ".join(p["slug"] for p in r["projects"]))'); do
    project="$(reg 'print(next(p["projectId"] for p in r["projects"] if p["slug"]==sys.argv[1]))' "$slug")"
    repo="$(reg 'print(next(p["repository"] for p in r["projects"] if p["slug"]==sys.argv[1]))' "$slug")"
    inst="$(repo_installation "$repo")"
    if [[ -z $inst ]]; then
      printf '%-12s %-30s app NOT installed\n' "$slug" "$repo"
      continue
    fi
    token="$(installation_token "$inst")"
    printf '%-12s %-30s installation %s; GitHub open issues %s; Kaneo %s\n' \
      "$slug" "$repo" "$inst" "$(open_issue_count "$repo" "$token")" "$(kaneo_counts "$project")"
  done
}

cmd_connect() {
  local slug="$1" project repo owner name inst result run status
  project="$(reg 'print(next(p["projectId"] for p in r["projects"] if p["slug"]==sys.argv[1]))' "$slug")"
  repo="$(reg 'print(next(p["repository"] for p in r["projects"] if p["slug"]==sys.argv[1]))' "$slug")"
  owner="${repo%%/*}" name="${repo#*/}"
  JWT="$(app_jwt)"
  inst="$(repo_installation "$repo")"
  if [[ -z $inst ]]; then
    echo "$repo: the developing.today app is not installed on it — install it first (see README)" >&2
    exit 1
  fi
  echo "$repo: covered by installation $inst"
  kaneo_login
  local ref="\"repositoryOwner\":\"$owner\",\"repositoryName\":\"$name\""
  kaneo -d "{\"projectId\":\"$project\",$ref}" "$KANEO_URL/api/github-integration/verify" |
    json 'print("verify:", d.get("message", d)); sys.exit(0 if d.get("hasRequiredPermissions") else 1)'
  kaneo -d "{$ref}" "$KANEO_URL/api/github-integration/project/$project" |
    json 'print("connect:", d.get("id") or d); sys.exit(0 if d.get("id") else 1)'
  run=""
  while :; do
    status=$(kaneo -o "$WORK/import.json" -w '%{http_code}' \
      -d "{\"projectId\":\"$project\"${run:+,\"runId\":\"$run\"}}" "$KANEO_URL/api/github-integration/import-issues")
    result="$(cat "$WORK/import.json")"
    case "$status" in
    200)
      echo "import: $(json 'print(d["imported"], "imported,", d["updated"], "updated,", d["skipped"], "skipped")' <<<"$result")"
      break
      ;;
    202) run="$(json 'print(d["runId"])' <<<"$result")" ;;
    429) sleep 1 ;;
    *)
      echo "import failed ($status): $result" >&2
      exit 1
      ;;
    esac
  done
  echo "GitHub open issues: $(open_issue_count "$repo" "$(installation_token "$inst")")"
  echo "Kaneo: $(kaneo_counts "$project")"
}

case "${1:-status}" in
status) cmd_status ;;
connect) cmd_connect "${2:?usage: $0 connect <slug>}" ;;
*)
  echo "usage: $0 status | connect <slug>" >&2
  exit 2
  ;;
esac
