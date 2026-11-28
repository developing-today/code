#!/usr/bin/env bash
# Regenerate the s11-relay spike's TLS material.
#
# The certificates are public and stay tracked; the private keys are
# generated locally by this script and are never committed (see .gitignore
# in this directory and README.md). Running this script (re)creates every
# key and certificate under gen/ and installs leafB as the live leaf,
# matching the topology recorded in ../RESULTS.md:
#
#   CA 1 (CN=s11 test CA 1) signs leafA and leafB; leafB is the live leaf
#   CA 2 (CN=s11 test CA 2) signs leafC (the wrong-CA / rotation leaf)
#   every leaf is CN=localhost with SAN DNS:localhost, IP:127.0.0.1
#   all keys are EC P-256, as in the original spike material
set -euo pipefail
cd "$(dirname "$0")"

DAYS=3650

mk_ca() { # file-stem subject
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
    -keyout "gen/$1.key" -out "gen/$1.pem" -days "$DAYS" \
    -subj "/CN=$2" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" \
    -addext "subjectKeyIdentifier=hash" \
    -addext "authorityKeyIdentifier=keyid:always"
  openssl x509 -in "gen/$1.pem" -outform DER -out "gen/$1.der"
}

mk_leaf() { # file-stem ca-stem
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
    -keyout "gen/$1.key" -out "gen/$1.csr" -subj "/CN=localhost"
  openssl x509 -req -in "gen/$1.csr" -CA "gen/$2.pem" -CAkey "gen/$2.key" \
    -CAcreateserial -out "gen/$1.pem" -days "$DAYS" -extfile gen/leaf.ext
}

mk_ca ca1 "s11 test CA 1"
mk_ca ca2 "s11 test CA 2"
mk_leaf leafA ca1
mk_leaf leafB ca1
mk_leaf leafC ca2

cp gen/leafB.pem live/leaf.pem
cp gen/leafB.key live/leaf.key
chmod 600 gen/*.key live/leaf.key
echo "generated: gen/{ca1,ca2,leafA,leafB,leafC} + live/leaf (copy of leafB)"
