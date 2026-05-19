const https = require("https");
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const VERSION = "0.11.0";
const PLATFORM = `${process.platform}-${process.arch}`;

const ASSETS = {
  "darwin-arm64": {
    url: `https://github.com/wickproject/wick/releases/download/v${VERSION}/wick-darwin-arm64.tar.gz`,
    sha256: "32bd4d71ef1738f295902c1f89ad2f2fc708bffbf2afbed721b4119f15f1b667",
  },
  "linux-x64": {
    url: `https://github.com/wickproject/wick/releases/download/v${VERSION}/wick-linux-amd64.tar.gz`,
    sha256: "475ce12c5511467794433f76eafb11a66e789aae3c34926b3ba9cfa506895760",
    hasLib: true,
  },
};

const asset = ASSETS[PLATFORM];
if (!asset) {
  console.error(
    `Wick does not yet have a prebuilt binary for ${PLATFORM}.\n` +
      `Supported: ${Object.keys(ASSETS).join(", ")}\n` +
      `See https://github.com/wickproject/wick for build-from-source instructions.`
  );
  process.exit(1);
}

const binDir = path.join(__dirname, "..", "bin");
const binPath = path.join(binDir, "wick");
const tarPath = path.join(binDir, "wick.tar.gz");

// Check if real binary exists (not the stub — stub is <500 bytes)
if (fs.existsSync(binPath) && fs.statSync(binPath).size > 1000) {
  process.exit(0);
}

fs.mkdirSync(binDir, { recursive: true });

function download(url, dest) {
  return new Promise((resolve, reject) => {
    const file = fs.createWriteStream(dest);
    https
      .get(url, (res) => {
        if (res.statusCode === 302 || res.statusCode === 301) {
          download(res.headers.location, dest).then(resolve).catch(reject);
          return;
        }
        if (res.statusCode !== 200) {
          reject(new Error(`Download failed: HTTP ${res.statusCode}`));
          return;
        }
        res.pipe(file);
        file.on("finish", () => file.close(resolve));
      })
      .on("error", reject);
  });
}

async function main() {
  console.log(`Downloading wick ${VERSION} for ${PLATFORM}...`);
  await download(asset.url, tarPath);

  execFileSync("tar", ["xzf", tarPath, "-C", binDir]);
  fs.unlinkSync(tarPath);
  fs.chmodSync(binPath, 0o755);

  // On Linux, the tarball includes libcronet.so — create a wrapper script
  // so LD_LIBRARY_PATH is set automatically
  const libPath = path.join(binDir, "libcronet.so");
  if (asset.hasLib && fs.existsSync(libPath)) {
    const realBin = path.join(binDir, "wick-bin");
    fs.renameSync(binPath, realBin);
    fs.writeFileSync(
      binPath,
      `#!/bin/sh\nLD_LIBRARY_PATH="${binDir}:$LD_LIBRARY_PATH" exec "${realBin}" "$@"\n`
    );
    fs.chmodSync(binPath, 0o755);
  }

  console.log("Wick installed successfully.");
}

main().catch((err) => {
  console.error(`Failed to install wick: ${err.message}`);
  process.exit(1);
});
