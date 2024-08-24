#!/usr/bin/env bash

# Set variables for key generation
NAME="User"
EMAIL="hi@developing-today.com"
PASSPHRASE="password"

# Create a temporary GPG batch file
cat >gpg-batch <<EOF
%echo Generating a basic OpenPGP key
Key-Type: RSA
Key-Length: 4096
Subkey-Type: RSA
Subkey-Length: 4096
Name-Real: $NAME
Name-Email: $EMAIL
Expire-Date: 0
Passphrase: $PASSPHRASE
# Do a commit here, so that we can later print "done" :-)
%commit
%echo done
EOF

# Generate the key using the batch file
gpg --batch --generate-key gpg-batch

# Remove the temporary batch file
rm gpg-batch

# Export the public key
gpg --armor --export $EMAIL > public_key.asc

echo "GPG key pair generated. Public key exported to public_key.asc"ch
