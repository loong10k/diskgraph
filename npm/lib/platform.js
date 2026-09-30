"use strict";

// Platform resolution is pure and exported so it is unit-testable on any
// host: the installer never guesses, it maps (platform, arch) to the exact
// release target triple, and refuses anything it has no binary for.

const TARGETS = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "win32-x64": "x86_64-pc-windows-msvc",
  // win32-arm64 builds are not produced yet; a clear refusal beats a
  // download that cannot exist.
};

function resolveTarget(platform, arch) {
  const key = `${platform}-${arch}`;
  return TARGETS[key] || null;
}

function archiveName(target, platform) {
  const ext = platform === "win32" ? "zip" : "tar.gz";
  return `diskgraph-${target}.${ext}`;
}

// The pinned version this installer expects. Kept in one place: the release
// workflow, the formula, and this file all move together per version.
const VERSION = "0.2.1";

function releaseBase() {
  return (
    process.env.DISKGRAPH_RELEASE_BASE ||
    "https://github.com/loong10k/diskgraph/releases/download"
  );
}

function assetUrl(target, platform) {
  const base = releaseBase().replace(/\/$/, "");
  return `${base}/v${VERSION}/${archiveName(target, platform)}`;
}

function checksumUrl(target, platform) {
  return `${assetUrl(target, platform)}.sha256`;
}

module.exports = { resolveTarget, archiveName, assetUrl, checksumUrl, VERSION, TARGETS };
