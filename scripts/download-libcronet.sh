#!/bin/bash
# Download prebuilt libcronet shared library from SagerNet releases.
# macOS prebuilts are not currently published by SagerNet.

set -euo pipefail

CRONET_VERSION="143.0.7499.109-2"
RELEASE_URL="https://github.com/SagerNet/cronet-go/releases/download/${CRONET_VERSION}"

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$ARCH" in
    x86_64)  ARCH="amd64" ;;
    aarch64) ARCH="arm64" ;;
    armv7l)  ARCH="arm" ;;
esac

case "$OS" in
    linux)
        LIB="libcronet-linux-${ARCH}.so"
        TARGET="libcronet.so"
        ;;
    mingw*|msys*|cygwin*)
        LIB="libcronet-windows-${ARCH}.dll"
        TARGET="libcronet.dll"
        ;;
    darwin)
        echo "Error: SagerNet does not publish prebuilt macOS Cronet libraries."
        echo ""
        echo "Options:"
        echo "  1. Build from Chromium source using sagernet/cronet-go's build-naive tool:"
        echo "     go run github.com/sagernet/cronet-go/cmd/build-naive@latest build"
        echo "  2. Test on Linux via Docker:"
        echo "     docker run --rm -v \$(pwd):/src -w /src golang:1.25 bash -c 'bash scripts/download-libcronet.sh . && make build'"
        exit 1
        ;;
    *)
        echo "Error: unsupported OS: $OS"
        exit 1
        ;;
esac

DEST="${1:-.}"
mkdir -p "$DEST" || { echo "Error: cannot create directory $DEST"; exit 1; }

echo "Downloading ${LIB}..."
curl -fL -o "${DEST}/${TARGET}" "${RELEASE_URL}/${LIB}"
echo "Saved to ${DEST}/${TARGET}"
echo ""
echo "To build wick: go build -tags with_purego -o wick ./cmd/wick"
echo "Place ${TARGET} next to the wick binary at runtime."
