#!/usr/bin/env bash
# Creates a self-signed code-signing certificate in the login keychain so that
# simpleKvm.app keeps a STABLE code identity across rebuilds. That makes the
# Accessibility (손쉬운 사용) permission grant persist — with ad-hoc signing the
# cdhash changes every build and the grant is silently invalidated.
#
# Run once:  ./scripts/create-signing-cert.sh
# The first time bundle-mac.sh signs with it, macOS may ask to allow codesign to
# use the key — click "Always Allow".
set -euo pipefail

CN="simpleKvm Self-Signed"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

if security find-certificate -c "$CN" "$KEYCHAIN" >/dev/null 2>&1; then
  echo "signing identity already exists: $CN"
  exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

cat > "$TMP/openssl.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = v3
prompt = no
[dn]
CN = $CN
[v3]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
EOF

openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout "$TMP/key.pem" -out "$TMP/cert.pem" \
  -days 3650 -config "$TMP/openssl.cnf" >/dev/null 2>&1

# -legacy: OpenSSL 3 defaults to a PKCS12 MAC macOS `security import` can't
# verify; legacy 3DES/RC2 + SHA1 MAC is compatible. Use a real password too.
openssl pkcs12 -export -legacy -inkey "$TMP/key.pem" -in "$TMP/cert.pem" \
  -out "$TMP/id.p12" -passout pass:simplekvm >/dev/null 2>&1

# Import key+cert; -T lets codesign use the key.
security import "$TMP/id.p12" -k "$KEYCHAIN" -P simplekvm -T /usr/bin/codesign

echo "created signing identity: $CN"
echo "now run ./scripts/bundle-mac.sh (it will sign with this identity)"
