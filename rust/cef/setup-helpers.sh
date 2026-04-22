#!/bin/bash
# Creates the CEF helper app bundles for macOS multi-process mode.
# All helpers use the same binary; only the bundle ID differs.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CEF_DIR="$(ls -d "$SCRIPT_DIR"/cef_binary_144* 2>/dev/null | head -1)"

if [ -z "$CEF_DIR" ]; then
    echo "Error: CEF distribution not found in $SCRIPT_DIR"
    exit 1
fi

DEST="${1:-$SCRIPT_DIR/Frameworks}"

echo "Setting up CEF helper bundles in $DEST..."

# Compile helper binary
echo "Compiling wick Helper..."
clang -DCEF_API_VERSION=14400 \
    -o "$SCRIPT_DIR/wick_helper" \
    "$SCRIPT_DIR/helper.m" \
    -I"$CEF_DIR" \
    -F"$CEF_DIR/Release" \
    -framework "Chromium Embedded Framework" \
    -framework Cocoa \
    -fobjc-arc

# Create Frameworks directory and link the CEF framework
mkdir -p "$DEST"
if [ ! -e "$DEST/Chromium Embedded Framework.framework" ]; then
    ln -sf "$CEF_DIR/Release/Chromium Embedded Framework.framework" "$DEST/"
fi

# Helper variants: "suffix:bundle_id_suffix"
HELPERS=(
    "::"
    " (GPU):.gpu"
    " (Renderer):.renderer"
    " (Alerts):.alerts"
    " (Plugin):.plugin"
)

for entry in "${HELPERS[@]}"; do
    IFS=: read -r suffix bid_suffix <<< "$entry"
    name="wick Helper${suffix}"
    bundle_id="dev.getwick.renderer.helper${bid_suffix}"
    bundle_dir="$DEST/${name}.app/Contents"

    echo "  Creating ${name}.app (${bundle_id})"
    mkdir -p "$bundle_dir/MacOS"

    # Use the same binary for all helpers (copy for first, hardlink for rest)
    if [ -z "$suffix" ]; then
        cp "$SCRIPT_DIR/wick_helper" "$bundle_dir/MacOS/$name"
    else
        ln -f "$DEST/wick Helper.app/Contents/MacOS/wick Helper" "$bundle_dir/MacOS/$name" 2>/dev/null \
            || cp "$SCRIPT_DIR/wick_helper" "$bundle_dir/MacOS/$name"
    fi

    # Info.plist
    cat > "$bundle_dir/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>${name}</string>
    <key>CFBundleIdentifier</key>
    <string>${bundle_id}</string>
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

    # PkgInfo
    echo -n "APPL????" > "$bundle_dir/PkgInfo"
done

# Clean up intermediate
rm -f "$SCRIPT_DIR/wick_helper"

echo "Done. Helper bundles created in $DEST"
echo ""
echo "Bundles:"
ls -d "$DEST"/*.app 2>/dev/null | while read -r d; do basename "$d"; done
