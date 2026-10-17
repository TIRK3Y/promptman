#!/usr/bin/env bash
set -euo pipefail

# PromptMan macOS Packaging Script
VERSION="${1:-1.0.0}"
ARCH="${2:-universal}" # "universal", "aarch64", or "x86_64"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
APP_BUNDLE="${DIST_DIR}/PromptMan.app"

echo "========================================"
echo " Packaging PromptMan v${VERSION} for macOS (${ARCH})"
echo "========================================"

mkdir -p "${DIST_DIR}"
cd "${ROOT_DIR}"

# 1. Build Binaries
if [ "${ARCH}" = "universal" ]; then
    echo "[1/4] Building Universal macOS binaries (Apple Silicon + Intel)..."
    rustup target add aarch64-apple-darwin x86_64-apple-darwin
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin

    mkdir -p "${ROOT_DIR}/target/universal"
    lipo -create -output "${ROOT_DIR}/target/universal/promptman" \
        "${ROOT_DIR}/target/aarch64-apple-darwin/release/promptman" \
        "${ROOT_DIR}/target/x86_64-apple-darwin/release/promptman"
    lipo -create -output "${ROOT_DIR}/target/universal/promptman-cli" \
        "${ROOT_DIR}/target/aarch64-apple-darwin/release/promptman-cli" \
        "${ROOT_DIR}/target/x86_64-apple-darwin/release/promptman-cli"

    PROMPTMAN_BIN="${ROOT_DIR}/target/universal/promptman"
    PROMPTMAN_CLI_BIN="${ROOT_DIR}/target/universal/promptman-cli"
elif [ "${ARCH}" = "aarch64" ]; then
    echo "[1/4] Building for Apple Silicon (aarch64)..."
    cargo build --release --target aarch64-apple-darwin
    PROMPTMAN_BIN="${ROOT_DIR}/target/aarch64-apple-darwin/release/promptman"
    PROMPTMAN_CLI_BIN="${ROOT_DIR}/target/aarch64-apple-darwin/release/promptman-cli"
else
    echo "[1/4] Building for Intel x86_64..."
    cargo build --release --target x86_64-apple-darwin
    PROMPTMAN_BIN="${ROOT_DIR}/target/x86_64-apple-darwin/release/promptman"
    PROMPTMAN_CLI_BIN="${ROOT_DIR}/target/x86_64-apple-darwin/release/promptman-cli"
fi

# 2. Assemble .app Bundle
echo "[2/4] Assembling PromptMan.app bundle..."
rm -rf "${APP_BUNDLE}"
mkdir -p "${APP_BUNDLE}/Contents/MacOS"
mkdir -p "${APP_BUNDLE}/Contents/Resources"

cp "${PROMPTMAN_BIN}" "${APP_BUNDLE}/Contents/MacOS/promptman"
cp "${PROMPTMAN_CLI_BIN}" "${APP_BUNDLE}/Contents/MacOS/promptman-cli"
chmod +x "${APP_BUNDLE}/Contents/MacOS/promptman"
chmod +x "${APP_BUNDLE}/Contents/MacOS/promptman-cli"

cp "${ROOT_DIR}/assets/macos/Info.plist" "${APP_BUNDLE}/Contents/Info.plist"
cp "${ROOT_DIR}/assets/icons/icon.icns" "${APP_BUNDLE}/Contents/Resources/icon.icns"

# 3. Create DMG Disk Image
echo "[3/4] Creating DMG disk image..."
DMG_NAME="PromptMan-v${VERSION}-macos-${ARCH}.dmg"
DMG_PATH="${DIST_DIR}/${DMG_NAME}"
DMG_TMP="${DIST_DIR}/dmg_tmp"

rm -rf "${DMG_TMP}" "${DMG_PATH}"
mkdir -p "${DMG_TMP}"
cp -R "${APP_BUNDLE}" "${DMG_TMP}/"
ln -s /Applications "${DMG_TMP}/Applications"

hdiutil create -volname "PromptMan" -srcfolder "${DMG_TMP}" -ov -format UDZO "${DMG_PATH}"
rm -rf "${DMG_TMP}"

# 4. Create tar.gz archive
echo "[4/4] Creating .tar.gz archive and checksums..."
TAR_NAME="PromptMan-v${VERSION}-macos-${ARCH}.tar.gz"
tar -czf "${DIST_DIR}/${TAR_NAME}" -C "${DIST_DIR}" "PromptMan.app"

# Generate Checksums
cd "${DIST_DIR}"
shasum -a 256 "${DMG_NAME}" "${TAR_NAME}" > "SHA256SUMS-macos-${ARCH}.txt"

echo "========================================"
echo " macOS Packaging Complete!"
echo " DMG:     ${DIST_DIR}/${DMG_NAME}"
echo " Archive: ${DIST_DIR}/${TAR_NAME}"
echo "========================================"
