#!/usr/bin/env bash
# idp-ca -- a small openssl-backed certificate authority for the self-hosted
# identity provider program (see README.md in this directory).
#
# State lives under IDP_CA_HOME (default ~/idp-ca):
#   root/     root CA: ca.key (encrypted), ca.crt, index.txt, serial, crl.pem
#   issuing/  issuing intermediate CA, same layout, plus chain.pem
#   issued/   leaf certificates issued by the issuing CA, one directory per name
#
# CA private keys are encrypted with the passphrase taken from the environment
# variable IDP_CA_PASSPHRASE. The passphrase is never taken from argv and never
# printed. Generate keys only on the CA host and escrow them to the secrets
# vault (see README.md, "Key handling").
set -euo pipefail

CA_HOME="${IDP_CA_HOME:-$HOME/idp-ca}"
ORG_DEFAULT="Developing Today"

usage() {
  cat <<'EOF'
Usage: idp-ca <command> [options]

Commands:
  init-root [--cn NAME] [--days N]
      Create the self-signed root CA (EC P-384, default 7300 days).
  create-intermediate [--cn NAME] [--days N]
      Create the issuing intermediate CA, signed by the root (default 3650 days).
  issue-leaf --profile PROFILE --name NAME [--cn NAME] [--san DNS:x]... [--days N]
      Issue a leaf from the issuing CA. Profiles:
        server        EC P-256, serverAuth, default 730 days, SAN defaults to DNS:<cn>
        client        EC P-256, clientAuth, default 730 days
        saml-signing  RSA 2048, digitalSignature, default 1095 days
        oidc-signing  RSA 2048, digitalSignature, default 1095 days
  revoke --name NAME
      Revoke an issued leaf (by its issued/ name) at the issuing CA.
  gencrl [--ca root|issuing]
      (Re)generate the CRL for a CA (default: issuing).
  inspect [--cert FILE]
      Show fingerprint, subject, issuer, serial and validity for a cert
      (default: both CA certs).
  verify-chain [--leaf FILE] [--with-crl]
      Verify the intermediate against the root, and a leaf against the chain.
      With --with-crl, revocation status is checked too.
  status
      Summarise the CA state under IDP_CA_HOME.
EOF
}

die() {
  echo "idp-ca: $*" >&2
  exit 1
}

need_pass() {
  if [ -z "${IDP_CA_PASSPHRASE:-}" ]; then
    die "IDP_CA_PASSPHRASE is not set; CA keys are encrypted, the passphrase is required"
  fi
}

ca_dir() {
  case "$1" in
  root) printf '%s\n' "$CA_HOME/root" ;;
  issuing) printf '%s\n' "$CA_HOME/issuing" ;;
  *) die "unknown CA: $1 (want root|issuing)" ;;
  esac
}

write_ca_config() {
  # $1 = ca dir, $2 = default digest
  local dir="$1"
  cat >"$dir/openssl.cnf" <<EOF
[ ca ]
default_ca = CA_default

[ CA_default ]
dir = $dir
database = \$dir/index.txt
new_certs_dir = \$dir/certs
serial = \$dir/serial
crlnumber = \$dir/crlnumber
certificate = \$dir/ca.crt
private_key = \$dir/ca.key
default_md = $2
default_crl_days = 30
policy = policy_any
unique_subject = no
copy_extensions = none

[ policy_any ]
commonName = supplied
organizationName = optional
countryName = optional
stateOrProvinceName = optional
localityName = optional
organizationalUnitName = optional
emailAddress = optional
EOF
}

setup_ca_dir() {
  local dir="$1" md="$2"
  mkdir -p "$dir/certs"
  chmod 700 "$dir"
  : >"$dir/index.txt"
  printf '1000\n' >"$dir/serial"
  printf '1000\n' >"$dir/crlnumber"
  write_ca_config "$dir" "$md"
}

extfile() {
  # $1 = profile; writes an extension file to stdout
  case "$1" in
  issuing)
    cat <<'EOF'
basicConstraints = critical, CA:true, pathlen:0
keyUsage = critical, keyCertSign, cRLSign
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid:always, issuer
EOF
    ;;
  server)
    cat <<'EOF'
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = serverAuth
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid, issuer
EOF
    ;;
  client)
    cat <<'EOF'
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = clientAuth
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid, issuer
EOF
    ;;
  signing)
    cat <<'EOF'
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature, nonRepudiation
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid, issuer
EOF
    ;;
  *) die "unknown extension profile: $1" ;;
  esac
}

cmd_init_root() {
  local cn="Developing Today IdP Root CA" days=7300
  while [ $# -gt 0 ]; do
    case "$1" in
    --cn)
      cn="$2"
      shift 2
      ;;
    --days)
      days="$2"
      shift 2
      ;;
    *) die "init-root: unknown option $1" ;;
    esac
  done
  need_pass
  local dir
  dir="$(ca_dir root)"
  [ -e "$dir/ca.crt" ] && die "root CA already exists at $dir"
  setup_ca_dir "$dir" sha384
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-384 \
    -aes-256-cbc -pass env:IDP_CA_PASSPHRASE -out "$dir/ca.key" 2>/dev/null
  chmod 600 "$dir/ca.key"
  openssl req -new -x509 -key "$dir/ca.key" -passin env:IDP_CA_PASSPHRASE \
    -subj "/CN=$cn/O=$ORG_DEFAULT" -days "$days" -sha384 \
    -addext "basicConstraints=critical,CA:true,pathlen:1" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" \
    -addext "subjectKeyIdentifier=hash" \
    -out "$dir/ca.crt"
  cmd_gencrl root
  echo "root CA created"
  cmd_inspect --cert "$dir/ca.crt"
}

cmd_create_intermediate() {
  local cn="Developing Today IdP Issuing CA" days=3650
  while [ $# -gt 0 ]; do
    case "$1" in
    --cn)
      cn="$2"
      shift 2
      ;;
    --days)
      days="$2"
      shift 2
      ;;
    *) die "create-intermediate: unknown option $1" ;;
    esac
  done
  need_pass
  local root_dir dir ext
  root_dir="$(ca_dir root)"
  dir="$(ca_dir issuing)"
  [ -e "$root_dir/ca.crt" ] || die "root CA not found; run init-root first"
  [ -e "$dir/ca.crt" ] && die "issuing CA already exists at $dir"
  setup_ca_dir "$dir" sha384
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-384 \
    -aes-256-cbc -pass env:IDP_CA_PASSPHRASE -out "$dir/ca.key" 2>/dev/null
  chmod 600 "$dir/ca.key"
  openssl req -new -key "$dir/ca.key" -passin env:IDP_CA_PASSPHRASE \
    -subj "/CN=$cn/O=$ORG_DEFAULT" -out "$dir/ca.csr"
  ext="$(mktemp)"
  extfile issuing >"$ext"
  openssl ca -config "$root_dir/openssl.cnf" -batch -notext \
    -passin env:IDP_CA_PASSPHRASE \
    -extfile "$ext" -in "$dir/ca.csr" -out "$dir/ca.crt" -days "$days"
  rm -f "$ext" "$dir/ca.csr"
  cat "$dir/ca.crt" "$root_dir/ca.crt" >"$dir/chain.pem"
  cmd_gencrl issuing
  echo "issuing CA created"
  cmd_inspect --cert "$dir/ca.crt"
}

cmd_issue_leaf() {
  local profile="" name="" cn="" days="" sans=()
  while [ $# -gt 0 ]; do
    case "$1" in
    --profile)
      profile="$2"
      shift 2
      ;;
    --name)
      name="$2"
      shift 2
      ;;
    --cn)
      cn="$2"
      shift 2
      ;;
    --san)
      sans+=("$2")
      shift 2
      ;;
    --days)
      days="$2"
      shift 2
      ;;
    *) die "issue-leaf: unknown option $1" ;;
    esac
  done
  [ -n "$profile" ] || die "issue-leaf: --profile is required"
  [ -n "$name" ] || die "issue-leaf: --name is required"
  [ -n "$cn" ] || cn="$name"
  need_pass
  local dir out ext keyalg extprofile
  dir="$(ca_dir issuing)"
  [ -e "$dir/ca.crt" ] || die "issuing CA not found; run create-intermediate first"
  out="$CA_HOME/issued/$name"
  [ -e "$out" ] && die "leaf already exists: $out"
  case "$profile" in
  server)
    keyalg="EC"
    extprofile="server"
    days="${days:-730}"
    ;;
  client)
    keyalg="EC"
    extprofile="client"
    days="${days:-730}"
    ;;
  saml-signing)
    keyalg="RSA"
    extprofile="signing"
    days="${days:-1095}"
    ;;
  oidc-signing)
    keyalg="RSA"
    extprofile="signing"
    days="${days:-1095}"
    ;;
  *) die "issue-leaf: unknown profile $profile" ;;
  esac
  if [ "$profile" = "server" ] && [ "${#sans[@]}" -eq 0 ]; then
    sans=("DNS:$cn")
  fi
  mkdir -p "$out"
  chmod 700 "$out"
  if [ "$keyalg" = "EC" ]; then
    openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 \
      -aes-256-cbc -pass env:IDP_CA_PASSPHRASE -out "$out/$name.key" 2>/dev/null
  else
    openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 \
      -aes-256-cbc -pass env:IDP_CA_PASSPHRASE -out "$out/$name.key" 2>/dev/null
  fi
  chmod 600 "$out/$name.key"
  openssl req -new -key "$out/$name.key" -passin env:IDP_CA_PASSPHRASE \
    -subj "/CN=$cn/O=$ORG_DEFAULT" -out "$out/$name.csr"
  ext="$(mktemp)"
  extfile "$extprofile" >"$ext"
  if [ "${#sans[@]}" -gt 0 ]; then
    local joined
    joined="$(printf '%s,' "${sans[@]}")"
    printf 'subjectAltName = %s\n' "${joined%,}" >>"$ext"
  fi
  openssl ca -config "$dir/openssl.cnf" -batch -notext \
    -passin env:IDP_CA_PASSPHRASE \
    -extfile "$ext" -in "$out/$name.csr" -out "$out/$name.crt" -days "$days"
  rm -f "$ext" "$out/$name.csr"
  cat "$out/$name.crt" "$dir/ca.crt" >"$out/$name.chain.pem"
  echo "leaf issued: profile=$profile name=$name days=$days"
  cmd_inspect --cert "$out/$name.crt"
}

cmd_revoke() {
  local name=""
  while [ $# -gt 0 ]; do
    case "$1" in
    --name)
      name="$2"
      shift 2
      ;;
    *) die "revoke: unknown option $1" ;;
    esac
  done
  [ -n "$name" ] || die "revoke: --name is required"
  need_pass
  local dir cert
  dir="$(ca_dir issuing)"
  cert="$CA_HOME/issued/$name/$name.crt"
  [ -e "$cert" ] || die "no such issued leaf: $cert"
  openssl ca -config "$dir/openssl.cnf" -passin env:IDP_CA_PASSPHRASE \
    -revoke "$cert"
  echo "revoked: $name (run 'idp-ca gencrl' to publish the CRL)"
}

cmd_gencrl() {
  local which="issuing"
  while [ $# -gt 0 ]; do
    case "$1" in
    --ca)
      which="$2"
      shift 2
      ;;
    root | issuing)
      which="$1"
      shift
      ;;
    *) die "gencrl: unknown option $1" ;;
    esac
  done
  need_pass
  local dir
  dir="$(ca_dir "$which")"
  openssl ca -config "$dir/openssl.cnf" -gencrl \
    -passin env:IDP_CA_PASSPHRASE -out "$dir/crl.pem"
  echo "CRL written: $dir/crl.pem"
}

cmd_inspect() {
  local cert=""
  while [ $# -gt 0 ]; do
    case "$1" in
    --cert)
      cert="$2"
      shift 2
      ;;
    *) die "inspect: unknown option $1" ;;
    esac
  done
  if [ -n "$cert" ]; then
    openssl x509 -in "$cert" -noout -fingerprint -sha256 -subject -issuer -serial -dates
    return
  fi
  local which dir
  for which in root issuing; do
    dir="$(ca_dir "$which")"
    if [ -e "$dir/ca.crt" ]; then
      echo "== $which CA =="
      openssl x509 -in "$dir/ca.crt" -noout -fingerprint -sha256 -subject -issuer -serial -dates
    fi
  done
}

cmd_verify_chain() {
  local leaf="" with_crl=0
  while [ $# -gt 0 ]; do
    case "$1" in
    --leaf)
      leaf="$2"
      shift 2
      ;;
    --with-crl)
      with_crl=1
      shift
      ;;
    *) die "verify-chain: unknown option $1" ;;
    esac
  done
  local root_dir dir
  root_dir="$(ca_dir root)"
  dir="$(ca_dir issuing)"
  echo "-- intermediate against root:"
  openssl verify -CAfile "$root_dir/ca.crt" "$dir/ca.crt"
  if [ -n "$leaf" ]; then
    if [ "$with_crl" -eq 1 ]; then
      local store
      store="$(mktemp)"
      cat "$root_dir/ca.crt" "$dir/ca.crt" "$root_dir/crl.pem" "$dir/crl.pem" >"$store"
      echo "-- leaf against chain, with CRL check:"
      openssl verify -crl_check -CAfile "$store" -untrusted "$dir/ca.crt" "$leaf"
      rm -f "$store"
    else
      echo "-- leaf against chain:"
      openssl verify -CAfile "$root_dir/ca.crt" -untrusted "$dir/ca.crt" "$leaf"
    fi
  fi
}

cmd_status() {
  echo "IDP_CA_HOME=$CA_HOME"
  local which dir count
  for which in root issuing; do
    dir="$(ca_dir "$which")"
    if [ -e "$dir/ca.crt" ]; then
      count="$(grep -c '^V' "$dir/index.txt" || true)"
      echo "$which CA: present, $count valid issued cert(s), CRL: $([ -e "$dir/crl.pem" ] && echo present || echo absent)"
    else
      echo "$which CA: absent"
    fi
  done
  if [ -d "$CA_HOME/issued" ]; then
    echo "leaves: $(find "$CA_HOME/issued" -name '*.crt' | wc -l) cert file(s) under issued/"
  fi
}

main() {
  local cmd="${1:-}"
  if [ $# -gt 0 ]; then shift; fi
  case "$cmd" in
  init-root) cmd_init_root "$@" ;;
  create-intermediate) cmd_create_intermediate "$@" ;;
  issue-leaf) cmd_issue_leaf "$@" ;;
  revoke) cmd_revoke "$@" ;;
  gencrl) cmd_gencrl "$@" ;;
  inspect) cmd_inspect "$@" ;;
  verify-chain) cmd_verify_chain "$@" ;;
  status) cmd_status "$@" ;;
  "" | -h | --help | help) usage ;;
  *) die "unknown command: $cmd (try 'idp-ca help')" ;;
  esac
}

main "$@"
