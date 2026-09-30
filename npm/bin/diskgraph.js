#!/usr/bin/env node
"use strict";

// Launcher: exec the platform binary that postinstall unpacked, passing
// argv and stdio straight through so JSON output and exit codes are the
// binary's own.
const { spawnSync } = require("child_process");
const path = require("path");
const { resolveTarget } = require("../lib/platform");

const target = resolveTarget(process.platform, process.arch);
const suffix = process.platform === "win32" ? ".exe" : "";
const binary = path.join(__dirname, "..", "vendor", `diskgraph-${target}`, "bin", `diskgraph${suffix}`);
const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error("diskgraph: binary not installed. Run: npx diskgraph-install");
  process.exit(1);
}
process.exit(result.status === null ? 1 : result.status);
