#!/usr/bin/env bash
# Stop and remove the Wick reference benchmark launchd job.

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "ERROR: uninstall.sh is macOS-specific."
    exit 1
fi

PLIST_DEST="$HOME/Library/LaunchAgents/com.wickproject.bench.plist"

if [[ ! -f "$PLIST_DEST" ]]; then
    echo "Bench job not installed (no plist at $PLIST_DEST)"
    exit 0
fi

launchctl unload "$PLIST_DEST" 2>/dev/null || true
rm "$PLIST_DEST"

echo "Bench job removed."
echo "Logs at \$HOME/.wick/bench/ are kept; delete by hand if desired."
