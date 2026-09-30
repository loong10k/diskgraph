"use strict";

// Platform resolution and URL construction, checked against the release
// workflow's target list: a typo here would send users to a 404.

const assert = require("assert");
const { resolveTarget, archiveName, assetUrl, VERSION } = require("../lib/platform");

const RELEASE_TARGETS = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "x86_64-unknown-linux-gnu",
  "aarch64-unknown-linux-gnu",
  "x86_64-pc-windows-msvc",
];

const PAIRS = [
  ["darwin", "arm64", "aarch64-apple-darwin"],
  ["darwin", "x64", "x86_64-apple-darwin"],
  ["linux", "x64", "x86_64-unknown-linux-gnu"],
  ["linux", "arm64", "aarch64-unknown-linux-gnu"],
  ["win32", "x64", "x86_64-pc-windows-msvc"],
];

for (const [platform, arch, expected] of PAIRS) {
  assert.strictEqual(
    resolveTarget(platform, arch),
    expected,
    `${platform}-${arch} must map to ${expected}`
  );
  assert.ok(
    RELEASE_TARGETS.includes(expected),
    `${expected} must exist in the release workflow matrix`
  );
}

// Unsupported combinations refuse rather than guess.
assert.strictEqual(resolveTarget("win32", "arm64"), null);
assert.strictEqual(resolveTarget("freebsd", "x64"), null);
assert.strictEqual(resolveTarget("linux", "riscv64"), null);

// Archive naming follows the release workflow (zip on Windows, tar.gz else).
assert.strictEqual(archiveName("x86_64-pc-windows-msvc", "win32"), "diskgraph-x86_64-pc-windows-msvc.zip");
assert.strictEqual(archiveName("aarch64-apple-darwin", "darwin"), "diskgraph-aarch64-apple-darwin.tar.gz");

// Asset URLs point at the pinned version's release.
const url = assetUrl("aarch64-apple-darwin", "darwin");
assert.ok(url.startsWith("https://github.com/loong10k/diskgraph/releases/download/v"));
assert.ok(url.endsWith(`v${VERSION}/diskgraph-aarch64-apple-darwin.tar.gz`), url);

console.log("diskgraph npm: all installer tests passed");
