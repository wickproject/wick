#!/bin/bash
# install-pro.sh: One-command Wick Pro installation for Linux servers.
#
# Usage:
#   WICK_KEY=wk_yourkey sudo -E bash <(curl -fsSL https://releases.getwick.dev/install-pro.sh)
#
# Or if you already have the script:
#   sudo WICK_KEY=wk_yourkey bash install-pro.sh
#
# Installs to /opt/wick/ with everything CEF needs.
# After install, run: sudo wick-tunnel init
#   → gives you a token to join from your laptop.

set -euo pipefail

WICK_DIR="/opt/wick"
CRONET_VERSION="143.0.7499.109-2"
CEF_VERSION="144.0.18+gc5b2ec2+chromium-144.0.7559.246"
CEF_PLATFORM="linux64"
# Resolve script directory before any cd
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd)" || SCRIPT_DIR=""

red()   { echo -e "\033[0;31m$*\033[0m"; }
green() { echo -e "\033[0;32m$*\033[0m"; }
bold()  { echo -e "\033[1m$*\033[0m"; }

# ── Check prerequisites ───────────────────────────────────────

if [[ "$(uname)" != "Linux" ]]; then
    red "This installer is for Linux servers only."
    echo "For macOS, use: brew tap wickproject/wick && brew install wick"
    exit 1
fi

ARCH=$(uname -m)
case "$ARCH" in
    x86_64)  CRONET_ARCH="linux-amd64" ;;
    aarch64) CRONET_ARCH="linux-arm64" ;;
    *) red "Unsupported architecture: $ARCH"; exit 1 ;;
esac

if [[ $EUID -ne 0 ]]; then
    red "Run with sudo: curl -fsSL https://releases.getwick.dev/install-pro.sh | sudo bash"
    exit 1
fi

# ── Create install directory ──────────────────────────────────

mkdir -p "$WICK_DIR"
cd "$WICK_DIR"

bold "Installing Wick Pro to $WICK_DIR..."
echo ""

# ── Step 1: Download wick binary ──────────────────────────────

if [[ ! -f "$WICK_DIR/wick" ]]; then
    echo "Downloading wick binary..."
    # Download from getwick.dev with API key (Pro customers)
    WICK_KEY="${WICK_KEY:-}"
    if [[ -n "$WICK_KEY" ]] && curl -fL -o wick-pro.tar.gz \
        "https://releases.getwick.dev/releases/${WICK_KEY}/wick-pro-linux-${ARCH}.tar.gz" < /dev/null 2>/dev/null \
        && tar tzf wick-pro.tar.gz &>/dev/null; then
        tar xzf wick-pro.tar.gz
        rm -f wick-pro.tar.gz
        chmod +x wick wick-renderer wick-helper 2>/dev/null || true
        echo "  Downloaded pre-built Wick Pro."
    else
        rm -f wick-pro.tar.gz 2>/dev/null
        # Fallback: build from source if repo checkout exists (developer use)
        if [[ -d /root/wick-pro/rust ]]; then
            echo "  Pre-built not available. Building from source..."
            source ~/.cargo/env 2>/dev/null || true
            if command -v cargo &>/dev/null; then
                cd /root/wick-pro/rust && cargo build --release --features cronet 2>&1 | tail -3
                cp target/release/wick "$WICK_DIR/wick"
                cp lib/linux_amd64/libcronet.so "$WICK_DIR/" 2>/dev/null || true
                strip "$WICK_DIR/wick" "$WICK_DIR/libcronet.so" 2>/dev/null || true
                cd "$WICK_DIR"
            else
                red "cargo not found. Install Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
                exit 1
            fi
        else
            if [[ -z "$WICK_KEY" ]]; then
                red "No API key provided and no local source checkout found."
                echo ""
                echo "  To install Wick Pro, set your API key:"
                echo "    WICK_KEY=wk_yourkey sudo bash install-pro.sh"
                echo ""
                echo "  Contact hello@getwick.dev for an API key."
            else
                red "Download failed. Your API key may be invalid or expired."
                echo "  Contact hello@getwick.dev for help."
            fi
            exit 1
        fi
    fi
else
    echo "wick binary already installed."
fi

# ── Step 2: Download libcronet ────────────────────────────────

if [[ ! -f "$WICK_DIR/libcronet.so" ]]; then
    # Copy from repo if building from source
    if [[ -f /root/wick-pro/rust/lib/linux_amd64/libcronet.so ]]; then
        cp /root/wick-pro/rust/lib/linux_amd64/libcronet.so "$WICK_DIR/"
        echo "Copied Cronet from repo."
    else
        echo "  Note: libcronet.so not found. Will use reqwest fallback (works but detectable TLS)."
    fi
else
    echo "libcronet.so already installed."
fi

# ── Step 3: Download CEF (Chromium Embedded Framework) ────────

CEF_DIR="cef_binary_${CEF_VERSION}_${CEF_PLATFORM}_minimal"

if [[ ! -f "$WICK_DIR/libcef.so" ]]; then
    # Primary: download pre-stripped CEF from our R2 (~115MB bz2, already stripped)
    if [[ -n "$WICK_KEY" ]] && curl -fL -o cef-runtime.tar.bz2 \
        "https://releases.getwick.dev/releases/${WICK_KEY}/cef-runtime-linux-${ARCH}.tar.bz2" 2>/dev/null \
        && tar tjf cef-runtime.tar.bz2 &>/dev/null; then
        echo "Downloading CEF runtime (~115MB, pre-stripped)..."
        tar xjf cef-runtime.tar.bz2
        rm -f cef-runtime.tar.bz2
        echo "  CEF runtime installed."
    else
        rm -f cef-runtime.tar.bz2 2>/dev/null
        # Fallback: download from Spotify CDN (unstripped, ~110MB bz2 → 1.5GB)
        echo "Downloading CEF from upstream (~110MB)..."
        curl -fL --progress-bar -o cef.tar.bz2 \
            "https://cef-builds.spotifycdn.com/cef_binary_${CEF_VERSION}_${CEF_PLATFORM}_minimal.tar.bz2"
        tar xjf cef.tar.bz2
        rm -f cef.tar.bz2

        # Copy and strip CEF runtime files
        echo "Setting up CEF runtime files..."
        cp -f "$CEF_DIR/Release/libcef.so" .
        cp -f "$CEF_DIR/Release/libEGL.so" . 2>/dev/null || true
        cp -f "$CEF_DIR/Release/libGLESv2.so" . 2>/dev/null || true
        cp -f "$CEF_DIR/Release/libvk_swiftshader.so" . 2>/dev/null || true
        cp -f "$CEF_DIR/Release/libvulkan.so.1" . 2>/dev/null || true
        cp -f "$CEF_DIR/Release/v8_context_snapshot.bin" .
        cp -f "$CEF_DIR/Resources/icudtl.dat" .
        cp -f "$CEF_DIR/Resources/resources.pak" .
        cp -f "$CEF_DIR/Resources/chrome_100_percent.pak" .
        cp -f "$CEF_DIR/Resources/chrome_200_percent.pak" .
        cp -rf "$CEF_DIR/Resources/locales" .
        strip libcef.so libEGL.so libGLESv2.so libvk_swiftshader.so libvulkan.so.1 2>/dev/null || true
        echo "  CEF stripped and installed."
    fi
else
    echo "CEF already installed."
fi

# ── Step 4: Build CEF renderer ────────────────────────────────

if [[ ! -f "$WICK_DIR/wick-renderer" ]]; then
    if command -v gcc &>/dev/null; then
        echo "Building CEF renderer..."

        # Check if source files exist (from git clone)
        RENDERER_SRC=""
        for src_dir in "$WICK_DIR/src" "/root/wick-pro/rust/cef" "${SCRIPT_DIR:+$SCRIPT_DIR/../rust/cef}"; do
            if [[ -f "$src_dir/renderer_linux.c" ]]; then
                RENDERER_SRC="$src_dir"
                break
            fi
        done

        if [[ -n "$RENDERER_SRC" ]]; then
            gcc -o wick-renderer "$RENDERER_SRC/renderer_linux.c" \
                -DCEF_API_VERSION=14400 \
                -I"$CEF_DIR" \
                -L. -lcef \
                -Wl,-rpath,'$ORIGIN' \
                -pthread

            gcc -o wick-helper "$RENDERER_SRC/helper_linux.c" \
                -DCEF_API_VERSION=14400 \
                -I"$CEF_DIR" \
                -L. -lcef \
                -Wl,-rpath,'$ORIGIN' \
                -pthread 2>/dev/null || true

            strip wick-renderer wick-helper 2>/dev/null || true
            echo "  Built wick-renderer and wick-helper."
        else
            echo "  CEF source not found. Downloading pre-built renderer..."
            curl -fL --progress-bar -o wick-renderer \
                "https://github.com/wickproject/wick-pro/releases/latest/download/wick-renderer-linux-$ARCH" \
                2>/dev/null || echo "  Pre-built renderer not available yet."
        fi
    else
        echo "  gcc not found. Install build-essential: apt install build-essential"
    fi
    chmod +x wick-renderer 2>/dev/null || true
    chmod +x wick-helper 2>/dev/null || true
fi

# ── Step 5: Build bindwg.so for WireGuard routing ─────────────

if [[ ! -f /usr/local/lib/bindwg.so ]] && command -v gcc &>/dev/null; then
    echo "Building WireGuard routing helper..."
    cat > /tmp/bindwg.c << 'CEOF'
#define _GNU_SOURCE
#include <dlfcn.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
int connect(int fd, const struct sockaddr *a, socklen_t l) {
    static int (*real)(int, const struct sockaddr *, socklen_t) = 0;
    if (!real) real = dlsym(RTLD_NEXT, "connect");
    if (a->sa_family == AF_INET) {
        struct sockaddr_in b = {.sin_family = AF_INET};
        inet_pton(AF_INET, "10.99.0.1", &b.sin_addr);
        bind(fd, (struct sockaddr *)&b, sizeof(b));
    }
    return real(fd, a, l);
}
CEOF
    gcc -shared -fPIC -o /usr/local/lib/bindwg.so /tmp/bindwg.c -ldl
    rm -f /tmp/bindwg.c
    echo "  Built /usr/local/lib/bindwg.so"
fi

# ── Step 6: Install wick-tunnel script ─────────────────────────

if [[ -n "$SCRIPT_DIR" ]] && [[ -f "$SCRIPT_DIR/wick-tunnel" ]]; then
    cp "$SCRIPT_DIR/wick-tunnel" /usr/local/bin/wick-tunnel
    chmod +x /usr/local/bin/wick-tunnel
    echo "  Installed wick-tunnel to /usr/local/bin/"
fi

# ── Step 7: Create wrapper script ─────────────────────────────
# This wrapper auto-sets LD_LIBRARY_PATH and LD_PRELOAD for
# WireGuard routing, so 'wick' just works.

# Remove any existing symlink first to avoid overwriting the binary
rm -f /usr/local/bin/wick

# Save WICK_KEY for runtime use (CAPTCHA solving, updates)
if [[ -n "$WICK_KEY" ]]; then
    echo "$WICK_KEY" > /opt/wick/.wick-key
    chmod 600 /opt/wick/.wick-key
fi

# Ensure yt-dlp for media downloads
if ! command -v yt-dlp &>/dev/null; then
    echo "Installing yt-dlp for media downloads..."
    pip3 install yt-dlp 2>/dev/null || apt-get install -y yt-dlp 2>/dev/null || true
fi

# Ensure Xvfb is installed (needed for WebGL in headless CEF)
if ! command -v Xvfb &>/dev/null; then
    echo "Installing Xvfb for WebGL support..."
    apt-get install -y xvfb 2>/dev/null || yum install -y xorg-x11-server-Xvfb 2>/dev/null || true
fi

cat > /usr/local/bin/wick << 'WRAPPER'
#!/bin/bash
export LD_LIBRARY_PATH="/opt/wick:${LD_LIBRARY_PATH:-}"

# Load API key for CAPTCHA solving proxy
if [[ -f /opt/wick/.wick-key ]]; then
    export WICK_KEY="$(cat /opt/wick/.wick-key)"
fi

# Timezone must match residential IP geolocation.
# Cache the timezone lookup (refreshed on tunnel reconnect).
if [[ -f /opt/wick/.wick-tz ]]; then
    export TZ="$(cat /opt/wick/.wick-tz)"
elif [[ -d /sys/class/net/wg-wick ]]; then
    # Look up timezone from residential IP via free geo API
    _TZ=$(curl -s --interface 10.99.0.1 --connect-timeout 3 http://worldtimeapi.org/api/ip 2>/dev/null | grep -oE '"timezone":"[^"]*"' | cut -d'"' -f4)
    if [[ -n "$_TZ" ]]; then
        echo "$_TZ" > /opt/wick/.wick-tz
        export TZ="$_TZ"
    fi
fi
export TZ="${TZ:-America/Denver}"

# Auto-route through WireGuard if tunnel is active
if [[ -d /sys/class/net/wg-wick ]] && [[ -f /usr/local/lib/bindwg.so ]]; then
    export LD_PRELOAD="/usr/local/lib/bindwg.so${LD_PRELOAD:+:$LD_PRELOAD}"
fi

exec /opt/wick/wick "$@"
WRAPPER
chmod +x /usr/local/bin/wick

# ── Step 9: Create systemd service ────────────────────────────

cat > /etc/systemd/system/wick.service << 'SERVICE'
[Unit]
Description=Wick Pro - Browser-grade web access for AI agents
After=network-online.target wg-quick@wg-wick.service
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/wick serve --mcp
Environment=HOME=/root
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
SERVICE

systemctl daemon-reload

# ── Done ──────────────────────────────────────────────────────

echo ""
green "=== Wick Pro Installed ==="
echo ""
echo "  Binary:    $WICK_DIR/wick"
echo "  Renderer:  $WICK_DIR/wick-renderer"
echo "  Wrapper:   /usr/local/bin/wick (auto-configures library paths)"
echo ""
bold "Next step: Set up residential IP routing"
echo ""
echo "  On this server:    sudo wick-tunnel init"
echo "                     → copy the token it prints"
echo ""
echo "  On your laptop:    sudo wick-tunnel join <token>"
echo "                     → your residential IP is now available"
echo ""
echo "  Then test:         wick fetch https://www.reddit.com/r/technology/ --no-robots"
echo ""
