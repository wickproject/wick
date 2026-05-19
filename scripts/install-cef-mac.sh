#!/bin/bash
# install-cef-mac.sh: Install the CEF renderer for Wick on macOS.
#
# Wick is fully open source; this installer fetches the CEF (Chromium Embedded
# Framework) runtime and builds the wick-renderer/wick-helper helpers from
# source. Required once per machine to enable JavaScript rendering.
#
# Usage (from a wick checkout):
#   bash scripts/install-cef-mac.sh
#
# Usage via `wick install cef`:
#   wick install cef   # runs this script for you
#
# Installs to ~/.wick/cef/ with the full CEF renderer bundle.

set -euo pipefail

WICK_DIR="$HOME/.wick/cef"
CEF_VERSION="144.0.18+gc5b2ec2+chromium-144.0.7559.246"
CEF_PLATFORM="macosarm64"

red()   { echo -e "\033[0;31m$*\033[0m"; }
green() { echo -e "\033[0;32m$*\033[0m"; }
bold()  { echo -e "\033[1m$*\033[0m"; }

if [[ "$(uname)" != "Darwin" ]]; then
    red "This installer is for macOS. For Linux, use install-cef-linux.sh"
    exit 1
fi

ARCH=$(uname -m)
if [[ "$ARCH" != "arm64" ]]; then
    red "Only Apple Silicon (arm64) is currently supported."
    exit 1
fi

# ── Create install directory ──────────────────────────────────

mkdir -p "$WICK_DIR"
cd "$WICK_DIR"

bold "Installing Wick Pro to $WICK_DIR..."
echo ""

# ── Step 0: Ensure yt-dlp for media downloads ────────────────

if ! command -v yt-dlp &>/dev/null; then
    echo "Installing yt-dlp for media downloads..."
    brew install yt-dlp 2>/dev/null || pip3 install yt-dlp 2>/dev/null || true
fi

# ── Step 1: Install wick binary (free tier via Homebrew) ──────

if ! command -v wick &>/dev/null; then
    echo "Installing wick via Homebrew..."
    brew tap wickproject/wick 2>/dev/null && brew install wick 2>/dev/null || {
        red "Homebrew install failed. Install manually: brew tap wickproject/wick && brew install wick"
        exit 1
    }
fi
echo "wick binary: $(which wick)"

# ── Step 2: Download CEF SDK ──────────────────────────────────

CEF_DIR="cef_binary_${CEF_VERSION}_${CEF_PLATFORM}_minimal"

if [[ ! -d "$WICK_DIR/$CEF_DIR" ]]; then
    WICK_KEY="${WICK_KEY:-}"
    # Try pre-stripped CEF from R2 first
    if [[ -n "$WICK_KEY" ]] && curl -fL -o cef-runtime.tar.bz2 \
        "https://releases.getwick.dev/releases/${WICK_KEY}/cef-runtime-macos-arm64.tar.bz2" < /dev/null 2>/dev/null \
        && tar tjf cef-runtime.tar.bz2 &>/dev/null; then
        echo "Downloading CEF runtime (pre-stripped)..."
        tar xjf cef-runtime.tar.bz2
        rm -f cef-runtime.tar.bz2
    else
        rm -f cef-runtime.tar.bz2 2>/dev/null
        echo "Downloading CEF from upstream (~120MB)..."
        curl -fL --progress-bar -o cef.tar.bz2 \
            "https://cef-builds.spotifycdn.com/cef_binary_${CEF_VERSION}_${CEF_PLATFORM}_minimal.tar.bz2" < /dev/null
        tar xjf cef.tar.bz2
        rm -f cef.tar.bz2
    fi
else
    echo "CEF already downloaded."
fi

# ── Step 3: Build renderer + helpers ──────────────────────────

APP_DIR="$WICK_DIR/wick-renderer.app"

# WICK_VERSION pins which ref to fetch source files from when running
# via `curl | bash` (no local checkout). Defaults to "main" for dev use.
# `wick install cef` passes the wick binary's own version.
WICK_VERSION="${WICK_VERSION:-main}"
if [[ "$WICK_VERSION" != "main" && "$WICK_VERSION" != v* ]]; then
    WICK_VERSION="v$WICK_VERSION"
fi
RAW_BASE="https://raw.githubusercontent.com/wickproject/wick/${WICK_VERSION}/rust/cef"

# Force-rebuild when the installed renderer is from a version before the
# current stdin protocol. Each protocol-changing release bumps the marker
# below; install-cef-mac.sh greps the installed binary for it and treats
# a missing marker as "stale, must rebuild" — keeps wick + renderer in
# sync without forcing all users to manually `rm -rf ~/.wick/cef`.
RENDERER_PROTOCOL="v2-selector"
RENDERER_BIN="$APP_DIR/Contents/MacOS/wick-renderer"
NEEDS_BUILD=0
if [[ ! -f "$RENDERER_BIN" ]]; then
    NEEDS_BUILD=1
elif ! grep -q "WICK_RENDERER_PROTOCOL=${RENDERER_PROTOCOL}" "$RENDERER_BIN" 2>/dev/null; then
    echo "Installed wick-renderer is from a pre-${RENDERER_PROTOCOL} build. Rebuilding..."
    rm -rf "$APP_DIR"
    NEEDS_BUILD=1
fi

if [[ "$NEEDS_BUILD" -eq 1 ]]; then
    echo "Building wick-renderer..."

    # Check for source files — prefer a local checkout, fall back to
    # fetching from the wick repo at WICK_VERSION.
    RENDERER_SRC=""
    for src_dir in "$WICK_DIR/src" "${BASH_SOURCE[0]:+$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/../rust/cef}"; do
        if [[ -f "$src_dir/renderer.m" ]]; then
            RENDERER_SRC="$src_dir"
            break
        fi
    done

    if [[ -z "$RENDERER_SRC" ]]; then
        echo "  No local source found. Fetching from ${RAW_BASE}..."
        RENDERER_SRC="$WICK_DIR/src"
        mkdir -p "$RENDERER_SRC"
        for f in renderer.m helper.m stealth.h setup-helpers.sh; do
            curl -fsSL -o "$RENDERER_SRC/$f" "$RAW_BASE/$f" || {
                red "Failed to fetch $f from $RAW_BASE"
                echo "  If you're offline or behind a proxy, clone the repo and run:"
                echo "    git clone https://github.com/wickproject/wick.git && cd wick && bash scripts/install-cef-mac.sh"
                exit 1
            }
        done
        chmod +x "$RENDERER_SRC/setup-helpers.sh"
    fi

    # Build renderer
    clang -o /tmp/wick-renderer-build "$RENDERER_SRC/renderer.m" \
        -DCEF_API_VERSION=14400 \
        -I"$WICK_DIR/$CEF_DIR" \
        -F"$WICK_DIR/$CEF_DIR/Release" \
        -framework "Chromium Embedded Framework" \
        -framework Cocoa \
        -rpath "@executable_path/../Frameworks" \
        -fobjc-arc

    # Create .app bundle
    mkdir -p "$APP_DIR/Contents/MacOS"

    # Info.plist for renderer
    cat > "$APP_DIR/Contents/Info.plist" << 'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>wick-renderer</string>
    <key>CFBundleIdentifier</key>
    <string>dev.getwick.renderer</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSUIElement</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
    <key>LSEnvironment</key>
    <dict>
        <key>MallocNanoZone</key>
        <string>0</string>
    </dict>
</dict>
</plist>
PLIST
    echo -n "APPL????" > "$APP_DIR/Contents/PkgInfo"
    mv /tmp/wick-renderer-build "$APP_DIR/Contents/MacOS/wick-renderer"

    # Build and set up helpers (pass CEF dir via symlink in source dir)
    echo "Building helper bundles..."
    ln -sf "$WICK_DIR/$CEF_DIR" "$RENDERER_SRC/$CEF_DIR" 2>/dev/null || true
    bash "$RENDERER_SRC/setup-helpers.sh" "$APP_DIR/Contents/Frameworks"
    rm -f "$RENDERER_SRC/$CEF_DIR" 2>/dev/null || true

    # Copy CEF framework into bundle (required for code signing)
    if [[ ! -e "$APP_DIR/Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework" ]]; then
        rm -rf "$APP_DIR/Contents/Frameworks/Chromium Embedded Framework.framework"
        cp -R "$WICK_DIR/$CEF_DIR/Release/Chromium Embedded Framework.framework" "$APP_DIR/Contents/Frameworks/"
    fi

    # Each helper .app needs a Frameworks/ symlink to the CEF framework
    # (their @executable_path/../Frameworks/ rpath looks in their own bundle)
    CEF_FW="$APP_DIR/Contents/Frameworks/Chromium Embedded Framework.framework"
    for helper in "$APP_DIR/Contents/Frameworks"/wick\ Helper*.app; do
        mkdir -p "$helper/Contents/Frameworks"
        ln -sf "$CEF_FW" "$helper/Contents/Frameworks/Chromium Embedded Framework.framework" 2>/dev/null
    done

    echo "  Built wick-renderer.app"
else
    echo "wick-renderer.app already installed."
fi

# ── Step 4: Save API key ──────────────────────────────────────

WICK_KEY="${WICK_KEY:-}"
if [[ -n "$WICK_KEY" ]]; then
    echo "$WICK_KEY" > "$WICK_DIR/.wick-key"
    chmod 600 "$WICK_DIR/.wick-key"
fi

# ── Step 5: Configure wick to find the renderer ───────────────
# The wick binary's cef.rs checks these paths for wick-renderer:
#   1. Next to the wick binary
#   2. ~/.wick/cef/wick-renderer.app/Contents/MacOS/wick-renderer
# Create symlink at path 2

mkdir -p "$HOME/.wick/cef"
ln -sf "$APP_DIR" "$HOME/.wick/cef/wick-renderer.app"

# ── Done ──────────────────────────────────────────────────────

echo ""
green "=== Wick Pro Installed (macOS) ==="
echo ""
echo "  Renderer:  $APP_DIR"
echo "  API key:   ~/.wick/pro/.wick-key"
echo "  Symlink:   ~/.wick/cef/wick-renderer.app"
echo ""
bold "Test it:"
echo ""
echo "  wick fetch https://www.reddit.com/r/Epstein/ --no-robots"
echo ""
echo "For residential IP routing from a cloud server:"
echo "  sudo curl -fsSL https://releases.getwick.dev/wick-tunnel -o /usr/local/bin/wick-tunnel"
echo "  sudo chmod +x /usr/local/bin/wick-tunnel"
echo "  sudo wick-tunnel join <token-from-server>"
echo ""
