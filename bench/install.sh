#!/usr/bin/env bash
# Install the Wick reference benchmark as a launchd job (macOS).
# Linux users: see README.md, this script is macOS-specific.

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "ERROR: install.sh is macOS-specific. For Linux, see bench/README.md."
    exit 1
fi

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOG_DIR="$HOME/.wick/bench"
PLIST_SRC="$REPO_DIR/bench/com.wickproject.bench.plist"
PLIST_DEST="$HOME/Library/LaunchAgents/com.wickproject.bench.plist"

if ! command -v wick >/dev/null 2>&1; then
    echo "ERROR: wick not found on PATH. Install it first:"
    echo "  brew install wickproject/wick/wick"
    echo "  # or: npm install -g wick-mcp"
    exit 1
fi

mkdir -p "$LOG_DIR"
mkdir -p "$(dirname "$PLIST_DEST")"

# Substitute @REPO_DIR@ and @LOG_DIR@ placeholders into the plist.
sed -e "s|@REPO_DIR@|$REPO_DIR|g" \
    -e "s|@LOG_DIR@|$LOG_DIR|g" \
    "$PLIST_SRC" > "$PLIST_DEST"

# Reload if already installed.
launchctl unload "$PLIST_DEST" 2>/dev/null || true
launchctl load "$PLIST_DEST"

# Run once now to populate the log immediately so the user can verify.
echo "Plist installed at $PLIST_DEST"
echo "Running first sweep now (this takes ~3-5 min)..."
launchctl start com.wickproject.bench

echo
echo "Status:"
launchctl list | grep -E '^[A-Z0-9-]+\s+\S+\s+com\.wickproject\.bench' || \
    echo "  (job loaded, will report status after first run completes)"
echo
echo "Logs: $LOG_DIR/sweep-*.log  (rotated, last 14 kept)"
echo "Schedule: every 1h"
echo "Stop with:  bash $REPO_DIR/bench/uninstall.sh"
