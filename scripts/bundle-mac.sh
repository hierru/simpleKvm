#!/usr/bin/env bash
# Builds the release binary and assembles simpleKvm.app under dist/.
# Run on macOS:  ./scripts/bundle-mac.sh
set -euo pipefail

cd "$(dirname "$0")/.."

APP_NAME="simpleKvm"
BUNDLE="dist/${APP_NAME}.app"
BIN="target/release/kvm-client"

echo "==> building release binary"
cargo build --release -p kvm-client

echo "==> assembling ${BUNDLE}"
rm -rf "${BUNDLE}"
mkdir -p "${BUNDLE}/Contents/MacOS"
mkdir -p "${BUNDLE}/Contents/Resources"
cp packaging/macos/Info.plist "${BUNDLE}/Contents/Info.plist"
cp "${BIN}" "${BUNDLE}/Contents/MacOS/kvm-client"
chmod +x "${BUNDLE}/Contents/MacOS/kvm-client"

# Prefer a stable self-signed identity (see create-signing-cert.sh) so the
# Accessibility permission grant survives rebuilds; fall back to ad-hoc, whose
# cdhash changes every build and invalidates the grant.
IDENTITY="simpleKvm Self-Signed"
if command -v codesign >/dev/null 2>&1; then
  if security find-certificate -c "${IDENTITY}" >/dev/null 2>&1; then
    echo "==> signing with ${IDENTITY}"
    codesign --force --deep --sign "${IDENTITY}" "${BUNDLE}"
  else
    echo "==> ad-hoc signing (run scripts/create-signing-cert.sh for a stable grant)"
    codesign --force --deep --sign - "${BUNDLE}"
  fi
fi

echo "==> done: ${BUNDLE}"
echo "Open with:  open ${BUNDLE}"
echo "First run: grant System Settings > Privacy & Security > Accessibility."
