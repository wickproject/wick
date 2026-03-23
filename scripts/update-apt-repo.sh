#!/bin/bash
# Updates the APT repository on the gh-pages branch.
# Called by the release workflow after building the .deb.
#
# Structure:
#   apt/
#     key.gpg              — public signing key
#     wick.list            — apt source file
#     dists/stable/
#       Release            — signed repo metadata
#       main/binary-amd64/
#         Packages          — package index
#         Packages.gz
#     pool/main/
#       wick_VERSION_amd64.deb

set -euo pipefail

DEB_FILE="$1"
VERSION="$2"

if [ -z "$DEB_FILE" ] || [ -z "$VERSION" ]; then
    echo "Usage: update-apt-repo.sh <deb-file> <version>"
    exit 1
fi

REPO_DIR=$(mktemp -d)
DIST_DIR="$REPO_DIR/apt/dists/stable/main/binary-amd64"
POOL_DIR="$REPO_DIR/apt/pool/main"

mkdir -p "$DIST_DIR" "$POOL_DIR"

# Copy .deb to pool
cp "$DEB_FILE" "$POOL_DIR/wick_${VERSION}_amd64.deb"

# Generate Packages index
cd "$REPO_DIR/apt"
dpkg-scanpackages pool/ /dev/null > "$DIST_DIR/Packages"
gzip -k "$DIST_DIR/Packages"

# Generate Release file
cat > "dists/stable/Release" << EOF
Origin: Wick
Label: Wick
Suite: stable
Codename: stable
Architectures: amd64
Components: main
Description: Browser-grade web access for AI agents
EOF

# Add checksums to Release
cd dists/stable
apt-ftparchive release . >> Release
cd "$REPO_DIR/apt"

# Sign Release if GPG key is available
if gpg --list-keys "hello@getwick.dev" >/dev/null 2>&1; then
    gpg --armor --detach-sign --output dists/stable/Release.gpg dists/stable/Release
    gpg --armor --clearsign --output dists/stable/InRelease dists/stable/Release
    gpg --armor --export hello@getwick.dev > key.gpg
fi

# Create the apt source list file
cat > wick.list << 'EOF'
deb [signed-by=/usr/share/keyrings/wick-archive-keyring.gpg] https://wickproject.github.io/wick/apt stable main
EOF

# Create install script
cat > install.sh << 'SCRIPT'
#!/bin/bash
# Install Wick from APT repository
set -e

echo "Adding Wick APT repository..."

# Add GPG key
curl -fsSL https://wickproject.github.io/wick/apt/key.gpg \
    | sudo gpg --dearmor -o /usr/share/keyrings/wick-archive-keyring.gpg

# Add repository
echo "deb [signed-by=/usr/share/keyrings/wick-archive-keyring.gpg] https://wickproject.github.io/wick/apt stable main" \
    | sudo tee /etc/apt/sources.list.d/wick.list > /dev/null

# Install
sudo apt update
sudo apt install -y wick

echo ""
echo "Wick installed! Run 'wick setup' to configure your AI coding tools."
SCRIPT
chmod +x install.sh

echo "APT repo prepared at $REPO_DIR/apt"
echo "Files:"
find "$REPO_DIR/apt" -type f | sort
