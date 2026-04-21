#!/bin/bash
# Build the CEF renderer and helper for Linux.
# Downloads CEF if not already present.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

CEF_VERSION="144.0.18+gc5b2ec2+chromium-144.0.7559.246"
CEF_PLATFORM="linux64"
CEF_DIR="cef_binary_${CEF_VERSION}_${CEF_PLATFORM}_minimal"
CEF_URL="https://cef-builds.spotifycdn.com/cef_binary_${CEF_VERSION}_${CEF_PLATFORM}_minimal.tar.bz2"

# Download CEF if needed
if [ ! -d "$CEF_DIR" ]; then
    echo "Downloading CEF for Linux (~110MB)..."
    curl -fL --progress-bar -o cef_linux.tar.bz2 "$CEF_URL"
    echo "Extracting..."
    tar xjf cef_linux.tar.bz2
    rm cef_linux.tar.bz2
fi

echo "Building wick-renderer..."
gcc -o wick-renderer renderer_linux.c \
    -DCEF_API_VERSION=14400 \
    -I"$CEF_DIR" \
    -L"$CEF_DIR/Release" \
    -lcef \
    -Wl,-rpath,'$ORIGIN' \
    -pthread

echo "Building wick-helper..."
gcc -o wick-helper helper_linux.c \
    -DCEF_API_VERSION=14400 \
    -I"$CEF_DIR" \
    -L"$CEF_DIR/Release" \
    -lcef \
    -Wl,-rpath,'$ORIGIN' \
    -pthread

echo ""
echo "Built successfully!"
echo "  wick-renderer  ($(du -h wick-renderer | cut -f1))"
echo "  wick-helper    ($(du -h wick-helper | cut -f1))"
echo ""
echo "To install, copy these alongside your wick binary:"
echo "  cp wick-renderer wick-helper /path/to/wick/"
echo ""
echo "Also copy CEF runtime files:"
echo "  cp $CEF_DIR/Release/libcef.so /path/to/wick/"
echo "  cp -r $CEF_DIR/Release/*.bin /path/to/wick/"
echo "  cp -r $CEF_DIR/Resources/* /path/to/wick/"
