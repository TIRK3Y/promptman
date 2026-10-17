#!/usr/bin/env bash
set -euo pipefail

# PromptMan Linux Packaging Script
VERSION="${1:-1.0.0}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
PKG_NAME="PromptMan-v${VERSION}-linux-x86_64"
PKG_DIR="${DIST_DIR}/${PKG_NAME}"

echo "========================================"
echo " Packaging PromptMan v${VERSION} for Linux (x86_64)"
echo "========================================"

mkdir -p "${DIST_DIR}"
cd "${ROOT_DIR}"

# 1. Build Binaries
echo "[1/4] Building release binaries..."
cargo build --release

PROMPTMAN_BIN="${ROOT_DIR}/target/release/promptman"
PROMPTMAN_CLI_BIN="${ROOT_DIR}/target/release/promptman-cli"

# 2. Assemble Portable Tarball
echo "[2/4] Assembling portable archive..."
rm -rf "${PKG_DIR}"
mkdir -p "${PKG_DIR}"

cp "${PROMPTMAN_BIN}" "${PKG_DIR}/promptman"
cp "${PROMPTMAN_CLI_BIN}" "${PKG_DIR}/promptman-cli"
chmod +x "${PKG_DIR}/promptman" "${PKG_DIR}/promptman-cli"

cp "${ROOT_DIR}/assets/linux/promptman.desktop" "${PKG_DIR}/"
cp "${ROOT_DIR}/assets/icons/app-icon.png" "${PKG_DIR}/promptman.png"
cp "${ROOT_DIR}/LICENSE" "${PKG_DIR}/LICENSE.txt"

cat << 'EOF' > "${PKG_DIR}/README.txt"
========================================================================
 PromptMan for Linux (x86_64)
========================================================================

HOW TO RUN:
-----------
1. Directly from terminal:
     ./promptman
   or for the CLI tool:
     ./promptman-cli --help

2. Install to your user environment (optional):
     chmod +x install.sh
     ./install.sh

REQUIREMENTS:
-------------
- glibc 2.31 or newer
- X11 or Wayland display server
- GNOME Keyring, KWallet, or keepassxc (Secret Service provider for secure API key storage)
========================================================================
EOF

cat << 'EOF' > "${PKG_DIR}/install.sh"
#!/usr/bin/env bash
set -e
INSTALL_BIN="${HOME}/.local/bin"
INSTALL_APPS="${HOME}/.local/share/applications"
INSTALL_ICONS="${HOME}/.local/share/icons/hicolor/512x512/apps"

mkdir -p "${INSTALL_BIN}" "${INSTALL_APPS}" "${INSTALL_ICONS}"
cp -f promptman "${INSTALL_BIN}/promptman"
cp -f promptman-cli "${INSTALL_BIN}/promptman-cli"
cp -f promptman.png "${INSTALL_ICONS}/promptman.png"
sed "s|Icon=promptman|Icon=${INSTALL_ICONS}/promptman.png|g" promptman.desktop > "${INSTALL_APPS}/promptman.desktop"
chmod +x "${INSTALL_APPS}/promptman.desktop"

echo "PromptMan installed to ${INSTALL_BIN}!"
echo "Make sure ${HOME}/.local/bin is in your PATH."
EOF
chmod +x "${PKG_DIR}/install.sh"

TAR_FILE="${DIST_DIR}/${PKG_NAME}.tar.gz"
tar -czf "${TAR_FILE}" -C "${DIST_DIR}" "${PKG_NAME}"
echo "Portable archive created: ${TAR_FILE}"

# 3. Build .deb Package (if dpkg-deb is available)
if command -v dpkg-deb &> /dev/null; then
    echo "[3/4] Building .deb package..."
    DEB_DIR="${DIST_DIR}/deb_tmp"
    rm -rf "${DEB_DIR}"
    mkdir -p "${DEB_DIR}/DEBIAN"
    mkdir -p "${DEB_DIR}/usr/bin"
    mkdir -p "${DEB_DIR}/usr/share/applications"
    mkdir -p "${DEB_DIR}/usr/share/icons/hicolor/512x512/apps"

    cat << EOF > "${DEB_DIR}/DEBIAN/control"
Package: promptman
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: amd64
Maintainer: PromptMan Developers <https://github.com/TIRK3Y/promptman>
Description: Desktop LLM workbench to compare cloud and local LLMs side by side.
 With real-time token, cost, and quality tracking.
EOF

    cp "${PROMPTMAN_BIN}" "${DEB_DIR}/usr/bin/promptman"
    cp "${PROMPTMAN_CLI_BIN}" "${DEB_DIR}/usr/bin/promptman-cli"
    chmod 755 "${DEB_DIR}/usr/bin/promptman" "${DEB_DIR}/usr/bin/promptman-cli"

    cp "${ROOT_DIR}/assets/linux/promptman.desktop" "${DEB_DIR}/usr/share/applications/promptman.desktop"
    cp "${ROOT_DIR}/assets/icons/app-icon.png" "${DEB_DIR}/usr/share/icons/hicolor/512x512/apps/promptman.png"

    DEB_FILE="${DIST_DIR}/PromptMan-v${VERSION}-linux-amd64.deb"
    dpkg-deb --build "${DEB_DIR}" "${DEB_FILE}"
    rm -rf "${DEB_DIR}"
    echo "Debian package created: ${DEB_FILE}"
else
    echo "[3/4] dpkg-deb not found, skipping .deb generation"
fi

# 4. Generate Checksums
echo "[4/4] Generating checksums..."
cd "${DIST_DIR}"
sha256sum "${PKG_NAME}.tar.gz" > "SHA256SUMS-linux.txt"
if [ -f "PromptMan-v${VERSION}-linux-amd64.deb" ]; then
    sha256sum "PromptMan-v${VERSION}-linux-amd64.deb" >> "SHA256SUMS-linux.txt"
fi

echo "========================================"
echo " Linux Packaging Complete!"
echo "========================================"
