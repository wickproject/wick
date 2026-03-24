const https = require("https");
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const VERSION = "0.4.0";
const PLATFORM = `${process.platform}-${process.arch}`;

const ASSETS = {
  "darwin-arm64": {
    url: `https://github.com/wickproject/wick/releases/download/v${VERSION}/wick-${VERSION}-darwin-arm64.tar.gz`,
    sha256: "567c4f83c523da30bab824f6a46b6c03ba58b332e65bd098e61b73c9d374811d",
  },
  "linux-x64": {
    url: `https://github.com/wickproject/wick/releases/download/v${VERSION}/wick-${VERSION}-linux-x86_64.tar.gz`,
    sha256: "0a9df30f3fdfc2e839411b94b40801dac7bc3cf74f01e1712e3732d586a7392d",
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

if (fs.existsSync(binPath)) {
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
  console.log("Wick installed successfully.");
}

main().catch((err) => {
  console.error(`Failed to install wick: ${err.message}`);
  process.exit(1);
});
