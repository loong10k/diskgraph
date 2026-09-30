# diskgraph (npm)

Thin installer for [DiskGraph](https://github.com/loong10k/diskgraph) — a
Rust file-relationship engine for AI agents and PruneX. This package ships
no binaries: `postinstall` downloads the archive for your platform from the
project's GitHub release, verifies its SHA-256, and unpacks the two
binaries. No compiler, no toolchain, no build.

```bash
npx -y diskgraph --version          # runs without installing
npm i -g diskgraph                  # puts `diskgraph` on PATH
```

## What you get

- `diskgraph` — the CLI: scope/index/query surfaces with JSON envelopes
- `diskgraph-mcp` — the MCP server (stdio, Streamable HTTP, legacy SSE)

## Quick start

```bash
diskgraph scope add --root ~/projects --data-dir ~/.diskgraph
diskgraph index --scope <scope-id> --data-dir ~/.diskgraph --wait
diskgraph tree --scope <scope-id> --data-dir ~/.diskgraph --depth 3
```

Point an MCP host at the server, e.g. for Codex:

```bash
codex mcp add diskgraph -- diskgraph-mcp --data-dir ~/.diskgraph --profile all
```

## Integrity

The installer downloads
`diskgraph-<target>.tar.gz` plus its published `.sha256` and refuses to
unpack anything whose digest does not match. Supported targets:
macOS (arm64, x64), Linux (x64, arm64), Windows (x64). An unsupported
platform is told to build from source (`cargo install --path
crates/diskgraph-cli`) rather than silently doing nothing.

## Other channels

Homebrew tap, GitHub releases, winget, scoop, and source builds cover the
same binaries; see the main repository's RELEASING.md.
