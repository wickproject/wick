#!/bin/bash
# Download prebuilt libcronet.a for macOS arm64 from the sagernet/cronet-go 'go' branch.
# This is a ~40MB static library built from Chromium 143.

set -euo pipefail

ARCH=$(uname -m)
# Pinned commit on the 'go' branch for reproducibility
COMMIT="2fef65f9dba9"
SHA256="828f9ce5595d3aa2c9be90e64e4e1b5f9120be70c101e1dbbdde4ef81cf67eed"

case "$ARCH" in
    arm64)  DIR="darwin_arm64" ;;
    *)      echo "Error: static libcronet.a is only supported for macOS arm64 (Apple Silicon)."
            echo "On Intel Macs, use 'make build-purego' with a libcronet.dylib instead."
            exit 1 ;;
esac

DEST="lib/${DIR}"
mkdir -p "$DEST"

if [ -f "$DEST/libcronet.a" ]; then
    echo "libcronet.a already exists at $DEST/libcronet.a"
    echo "Verifying checksum..."
    if echo "$SHA256  $DEST/libcronet.a" | shasum -a 256 -c --status 2>/dev/null; then
        echo "Checksum OK."
        exit 0
    else
        echo "Checksum mismatch — re-downloading."
        rm -f "$DEST/libcronet.a"
    fi
fi

echo "Downloading libcronet.a for macOS arm64 (~40MB)..."
curl -fL --progress-bar -o "$DEST/libcronet.a" \
    "https://raw.githubusercontent.com/SagerNet/cronet-go/${COMMIT}/lib/${DIR}/libcronet.a"

echo "Verifying checksum..."
if ! echo "$SHA256  $DEST/libcronet.a" | shasum -a 256 -c --status 2>/dev/null; then
    echo "Error: SHA256 checksum verification failed!"
    echo "Expected: $SHA256"
    echo "Got:      $(shasum -a 256 "$DEST/libcronet.a" | cut -d' ' -f1)"
    rm -f "$DEST/libcronet.a"
    exit 1
fi

echo "Checksum OK."
echo "Saved to $DEST/libcronet.a"
echo ""
echo "Build with: make build"
