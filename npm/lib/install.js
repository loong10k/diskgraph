"use strict";

// Thin installer: download this platform's archive from the GitHub release,
// verify its SHA-256 against the published digest, unpack the two binaries,
// and mark them executable. Deliberately dependency-free (npm's thin-installer
// pattern) so `npx diskgraph` never compiles anything and never needs a
// toolchain.

const fs = require("fs");
const os = require("os");
const path = require("path");
const https = require("https");
const http = require("http");
const { execFileSync } = require("child_process");
const { pipeline } = require("stream/promises");
const crypto = require("crypto");
const { resolveTarget, assetUrl, checksumUrl, VERSION } = require("./platform");

const ROOT = path.join(__dirname, "..");
const VENDOR = path.join(ROOT, "vendor");

function fetchWithRedirects(url, redirectsLeft = 5) {
  // GitHub release asset URLs redirect to objects.githubusercontent.com; the
  // digest check is the real integrity gate, so following redirects is safe.
  // Plain http is only reachable when an explicit base URL says so (local
  // mirrors and the install drill); the default release base is https.
  const client = url.startsWith("http://") ? http : https;
  return new Promise((resolve, reject) => {
    client
      .get(url, (response) => {
        if (response.statusCode >= 300 && response.statusCode < 400 && response.headers.location) {
          if (redirectsLeft === 0) return reject(new Error("too many redirects"));
          response.resume();
          return resolve(fetchWithRedirects(response.headers.location, redirectsLeft - 1));
        }
        if (response.statusCode !== 200) {
          response.resume();
          return reject(new Error(`GET ${url} -> ${response.statusCode}`));
        }
        resolve(response);
      })
      .on("error", reject);
  });
}

async function readText(url) {
  const response = await fetchWithRedirects(url);
  const chunks = [];
  for await (const chunk of response) chunks.push(chunk);
  return Buffer.concat(chunks).toString("utf8");
}

async function download(url, destination) {
  const response = await fetchWithRedirects(url);
  // Node's IncomingMessage is already a readable stream; no conversion.
  await pipeline(response, fs.createWriteStream(destination));
}

async function main() {
  if (process.env.DISKGRAPH_SKIP_DOWNLOAD === "1") {
    console.log("diskgraph: DISKGRAPH_SKIP_DOWNLOAD=1, nothing installed");
    return;
  }
  const target = resolveTarget(process.platform, process.arch);
  if (!target) {
    console.error(
      `diskgraph: no prebuilt binary for ${process.platform}-${process.arch}. ` +
        `Build from source instead: cargo install diskgraph-cli`
    );
    process.exit(1);
  }
  fs.mkdirSync(VENDOR, { recursive: true });
  const archive = path.join(VENDOR, path.basename(assetUrl(target, process.platform)));
  console.log(`diskgraph ${VERSION}: fetching ${target}`);
  try {
    await download(assetUrl(target, process.platform), archive);
    const claimed = (await readText(checksumUrl(target, process.platform)))
      .trim()
      .split(/\s+/)[0];
    const actual = crypto.createHash("sha256").update(fs.readFileSync(archive)).digest("hex");
    if (actual !== claimed) {
      fs.rmSync(archive, { force: true });
      throw new Error(`checksum mismatch: release says ${claimed}, download is ${actual}`);
    }
    console.log(`diskgraph ${VERSION}: checksum verified (${actual.slice(0, 16)}...)`);

    if (process.platform === "win32") {
      execFileSync("powershell", ["-NoProfile", "-Command",
        `Expand-Archive -Force -Path '${archive}' -DestinationPath '${VENDOR}'`]);
    } else {
      execFileSync("tar", ["-xzf", archive, "-C", VENDOR]);
    }
    const binDir = path.join(VENDOR, `diskgraph-${target}`, "bin");
    for (const name of ["diskgraph", "diskgraph-mcp"]) {
      const binary = path.join(binDir, name);
      if (fs.existsSync(binary)) fs.chmodSync(binary, 0o755);
    }
    fs.rmSync(archive, { force: true });
    console.log("diskgraph: installed");
  } catch (error) {
    // A failed install must not break `npm install` of a project that
    // merely depends on something else; it explains itself and exits 0.
    console.error(`diskgraph: install failed (${error.message}).`);
    console.error("diskgraph: build from source with: cargo install diskgraph-cli");
  }
}

if (require.main === module) {
  main().catch((error) => {
    console.error(`diskgraph: ${error.message}`);
    process.exit(0);
  });
}

module.exports = { main };
