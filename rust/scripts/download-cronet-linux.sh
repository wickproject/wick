#!/bin/bash
# Download prebuilt libcronet.so for Linux from SagerNet releases.
set -euo pipefail

CRONET_VERSION="143.0.7499.109-2"
RELEASE_URL="https://github.com/SagerNet/cronet-go/releases/download/${CRONET_VERSION}"

ARCH=$(uname -m)
case "$ARCH" in
    x86_64)  DIR="linux_amd64"; LIB="libcronet-linux-amd64.so" ;;
    aarch64) DIR="linux_arm64"; LIB="libcronet-linux-arm64.so" ;;
    *)       echo "Error: unsupported architecture: $ARCH"; exit 1 ;;
esac

DEST="lib/${DIR}"
mkdir -p "$DEST"

if [ -f "$DEST/libcronet.so" ]; then
    echo "libcronet.so already exists at $DEST/"
    exit 0
fi

echo "Downloading ${LIB} for Linux ${ARCH}..."
curl -fL --progress-bar -o "$DEST/libcronet.so" "${RELEASE_URL}/${LIB}"
echo "Saved to $DEST/libcronet.so"
