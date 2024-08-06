#!/usr/bin/env bash

set -exuo pipefail
# "NS" # DO NOT DELETE THIS
# "SOA" # CAN ONLY DELETE BY ID
types=("A" "AAAA" "ALIAS" "CNAME" "MX" "PTR" "TXT")
subdomains=("www" "blog" "mail")

dir="$(dirname -- "$(which -- "$0" 2>/dev/null || realpath -- "$0")")"
echo "dir: $dir"

if [ -z "${1:-}" ]; then
  domains_file="$dir/domains.txt"
  echo "domains_file: $domains_file"

  if [ ! -f "$domains_file" ]; then
    echo "File not found: $domains_file"
    exit 1
  fi
  domains=$(cat "$domains_file")
else
  domains="$1"
fi

repo_root="$(git rev-parse --show-toplevel)"
sops_relative_path="lib/config.enc/common/porkbun.yaml"
sops_file="$repo_root/$sops_relative_path"

if [ ! -f "$sops_file" ]; then
  echo "File not found: $sops_file"
  exit 1
fi

if ! command -v sops &> /dev/null; then
  echo "sops not found"
  exit 1
fi

set +x

sops_yaml=$(sops -d "$sops_file")

if ! command -v yq &> /dev/null; then
  echo "yq not found"
  exit 1
fi

porkbun_api_key=$(yq -r '.porkbun_api_key' <<< "$sops_yaml")
porkbun_secret_key=$(yq -r '.porkbun_secret_key' <<< "$sops_yaml")

if [ -z "$porkbun_api_key" ]; then
  echo "porkbun_api_key not found"
  exit 1
fi

if [ -z "$porkbun_secret_key" ]; then
  echo "porkbun_secret_key not found"
  exit 1
fi

set -x

# Retrieve DNS Records by Domain
# URI Endpoint: https://api.porkbun.com/api/json/v3/dns/retrieve/DOMAIN
# {
# 	"secretapikey": "YOUR_SECRET_API_KEY",
# 	"apikey": "YOUR_API_KEY"
# }
# {
# 	"status": "SUCCESS",
# 	"records": [
# 		{
# 			"id": "106926659",
# 			"name": "www.borseth.ink",
# 			"type": "A",
# 			"content": "1.1.1.1",
# 			"ttl": "600",
# 			"prio": "0",
# 			"notes": ""
# 		}
# 	]
# }
# Delete DNS Records by Domain, Type, and Subdomain
# https://api.porkbun.com/api/json/v3/dns/deleteByNameType/DOMAIN/TYPE/[SUBDOMAIN]#
# {
# 	"secretapikey": "YOUR_SECRET_API_KEY",
# 	"apikey": "YOUR_API_KEY"
# }
# {
# 	"status": "SUCCESS"
# }
if [ -f "$dir/.lock" ]; then
  echo "lock file exists, delete it to continue"
  exit 1
fi
touch "$dir/.lock"
function cleanup() {
  echo "cleaning up"
  echo "deleting lock file"
  rm -f "$dir/.lock"
  # echo "saving tfstate"
  # $dir/save.sh
  # echo "successfully saved tfstate"
  echo "done cleaning up"
}
trap cleanup EXIT
run_id=$(date +%s)
run_dir="$dir/logs/records/$run_id"
mkdir -p "$run_dir"
for domain in $domains; do
  echo "domain: $domain"
  inner_run_id=$(date +%s)
  retrieve_response=$(curl -X POST "https://api.porkbun.com/api/json/v3/dns/retrieve/$domain" \
    -H "Content-Type: application/json" \
    -d "{\"secretapikey\":\"$porkbun_secret_key\",\"apikey\":\"$porkbun_api_key\"}")
  echo "retrieve_response:\n$(jq . <<< "$retrieve_response")"
  jq . <<< "$retrieve_response" >> "$run_dir/$run_id.$inner_run_id.retrieve.$domain.json"
  sleep 2
done
for domain in $domains; do
  echo "domain: $domain"
  for type in "${types[@]}"; do
  echo "domain: $domain"
  echo "type: $type"
    inner_run_id=$(date +%s)
    delete_response=$(curl -X POST "https://api.porkbun.com/api/json/v3/dns/deleteByNameType/$domain/$type" \
      -H "Content-Type: application/json" \
      -d "{\"secretapikey\":\"$porkbun_secret_key\",\"apikey\":\"$porkbun_api_key\"}")
    echo "delete_response:\n$(jq . <<< "$delete_response")"
    jq . <<< "$delete_response" >> "$run_dir/$run_id.$inner_run_id.delete.$domain.$type.json"
    sleep 2
  done

  for subdomain in "${subdomains[@]}"; do
    echo "domain: $domain"
    echo "subdomain: $subdomain"
    for type in "${types[@]}"; do
      echo "domain: $domain"
      echo "subdomain: $subdomain"
      echo "type: $type"
      inner_run_id=$(date +%s)
      delete_response=$(curl -X POST "https://api.porkbun.com/api/json/v3/dns/deleteByNameType/$domain/$type/$subdomain" \
        -H "Content-Type: application/json" \
        -d "{\"secretapikey\":\"$porkbun_secret_key\",\"apikey\":\"$porkbun_api_key\"}")
      echo "delete_response:\n$(jq . <<< "$delete_response")"
      jq . <<< "$delete_response" >> "$run_dir/$run_id.$inner_run_id.delete.$domain.$type.$subdomain.json"
      sleep 2
    done
    sleep 2
  done
  sleep 2
done
