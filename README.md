<div align="center">

<img src="docs/assets/logo.svg" alt="DiskGraph" height="72" />

# DiskGraph

**A file-relationship engine for AI agents — disk usage, ownership, evidence, and history, indexed locally**

`npx -y diskgraph --help` needs nothing on your machine. Everything stays in one directory on your disk.

</div>

[English](README.md) | [简体中文](README.zh-CN.md)

[![crates.io](https://img.shields.io/badge/crates.io-diskgraph--cli-blue)](https://crates.io/crates/diskgraph-cli)
[![npm](https://img.shields.io/badge/npm-diskgraph-blue)](https://www.npmjs.com/package/diskgraph)
[![Homebrew](https://img.shields.io/badge/brew-loong10k%2Fdiskgraph%2Fdiskgraph-blue)](https://github.com/loong10k/homebrew-diskgraph)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

[![macOS](https://img.shields.io/badge/macOS-supported-lightgrey)](#install) [![Linux](https://img.shields.io/badge/Linux-supported-lightgrey)](#install) [![Windows](https://img.shields.io/badge/Windows-supported-lightgrey)](#install)
[![Codex](https://img.shields.io/badge/Codex-CLI-blueviolet)](#connect-an-agent) [![Claude Code](https://img.shields.io/badge/Claude_Code-blueviolet)](#connect-an-agent)

---

## See it

Disk usage is a picture, not a number. Three surfaces draw the same map from the same index, so what you see in a terminal, a browser, or an agent's reply never disagrees.

<table>
<tr>
<td width="62%"><img src="docs/assets/treemap-html.png" alt="DiskGraph browser treemap" /></td>
<td valign="top" width="38%">

**Browser** — one self-contained file, no CDN, no build step:

```bash
diskgraph tree --scope <id> --html usage.html
```

Click a block to descend, click the background to go up. Hue is the category a collector assigned; brightness is the share of the parent.

</td>
</tr>
<tr>
<td valign="top" width="62%">

**Terminal** — walks one directory level at a time, so a four-million-node index opens instantly:

```bash
diskgraph tui --scope <id>
```

<pre>
 demo   464 MiB  10 files   ↑↓ move · enter descend · esc up · s sort · q quit
┌ disk usage · rev-ceb45ad0────────────────────────────────────────────┐┌ selection ──────┐
│ ┌atarget.bin 180 MiB───────beta 90.0 MiB──gamma 4…┐LCaches 55.0 MiB  …  ││workspaces      │
│ │                                                 │                  ││size   310 MiB  │
│ │                                                 │                  ││files  3        │
│ │                                                 │                  ││kind   directory│
│ │                                                 │                  ││category Code   │
└──────────────────────────────────────────────────┴──────────────────┘└────────────────┘
</pre>

</td>
<td valign="top" width="38%">

**Agent** — the same map as text, inside the conversation:

```text
diskgraph_top {"scope":"…","format":"treemap"}
```

<pre>
      SIZE      TOTAL    FILES
──────────────────────────────────────────────
▎workspaces ███████████████████████████   310 MiB      3  Code
 Library    ████████▌                  55.0 MiB      1  Cache
 Media      ████▎                      25.0 MiB      1  Other

3 entries · 390 MiB in 5 files
</pre>

</td>
</tr>
</table>

## Why

| You have | Its limit | DiskGraph |
| :--- | :--- | :--- |
| `du`, `ncdu` | one tree, no history, re-walks every run | persistent snapshots; compare two points in time |
| Finder / Explorer | pretty, but not queryable by an agent | bounded JSON that the same engine answers from |
| disktree | beautiful, human-only | the same squarified map, plus CLI / MCP / FFI over one index |
| An agent with a shell | burns context listing directories | typed relations, evidence, and a map in one call |

DiskGraph is a **graph, not a viewer**. Every directory is a node with typed relations — owned by a project, rebuilt by a tool, protected — and every answer points at the evidence that produced it.

## Install

```bash
# macOS
brew install loong10k/diskgraph/diskgraph

# anywhere, no toolchain needed
npx -y diskgraph --version

# Debian / Ubuntu  (from the GitHub release)
sudo dpkg -i diskgraph_0.2.1_amd64.deb

# Fedora / RHEL    (from the GitHub release)
sudo dnf install diskgraph-0.2.1-1.x86_64.rpm

# Windows
winget install loong10k.DiskGraph          # or: scoop install diskgraph

# from source
cargo install diskgraph-cli
```

The CLI and the MCP server (`diskgraph-mcp`) install together. Prebuilt binaries need no Rust; building from source needs 1.97+.

## Get started

```bash
# 1. register a directory to watch (prints a scope id)
diskgraph scope add --root ~/projects --data-dir ~/.diskgraph

# 2. index it and wait for the walk to publish a revision
diskgraph index --scope <scope-id> --data-dir ~/.diskgraph --wait

# 3. look at it
diskgraph top  --scope <scope-id> --data-dir ~/.diskgraph
diskgraph tree --scope <scope-id> --data-dir ~/.diskgraph --depth 4 --html usage.html
diskgraph tui  --scope <scope-id> --data-dir ~/.diskgraph
```

Every command answers with a machine-readable envelope under `--json`, and every error carries a stable code (see [`RELEASING.md`](RELEASING.md) and `diskgraph --help`).

## Connect an agent

```bash
# Codex CLI
codex mcp add diskgraph -- diskgraph-mcp --data-dir ~/.diskgraph --profile all

# Claude Code, Cursor, or any MCP host
claude mcp add diskgraph -- diskgraph-mcp --data-dir ~/.diskgraph --profile all
```

Eighteen read tools today; profiles (`read-minimal`, `read-full`, `manage`, `all`) decide which are listed. A non-loopback HTTP bind requires a signed token, and every connection is body-, rate-, and quota-limited.

## Measured

On an Apple Silicon host, against a real home directory (4,526,858 files, 261 GiB observed):

| Measurement | |
| :--- | :--- |
| Full index: walk, stage, publish | 4 m 17 s |
| Tree render, `--depth 3`, structured read path | 10.1 s |
| Load path after the v4 structured-column change | 2.4× faster |
| Peak memory during a tree render | bounded by the rendered depth, not the index size |

These are measurements from this repository's acceptance records ([`docs/acceptance/`](docs/acceptance/)), not targets or estimates.

## Safety

- **Read-only by default.** The catalog commands that could move or delete a file answer `unsupported` until the review surface ships; nothing is quietly enabled.
- **Local only.** Everything happens inside `--data-dir` (two SQLite files). Nothing phones home, and the HTML report has no external references at all.
- **Honest coverage.** A walk that hits a budget, a depth limit, or an unreadable directory says so; a truncated view never renders like a complete one.
- **Separate data and control.** The graph database is rebuildable by rescanning; the control database holds scopes, policies, and operation history and is never touched by retention.
- **Unsigned today.** macOS Gatekeeper and Windows SmartScreen will warn until signing is configured — a recorded gap, not an oversight.

## Under the hood

Ten crates, one engine. `core` holds the model, the bounded queries, and the squarified layout the three surfaces share; `store` keeps snapshots and control data apart; `engine` orchestrates durable jobs and the authorized surface; `ops` plans and executes approval-gated operations; `mcp` speaks the protocol over stdio, Streamable HTTP, and legacy SSE; `ffi` reaches Swift and Kotlin.

The scanner is [disktree](https://github.com/tobi/disktree)'s, vendored at a pinned revision whose per-file digests are verified on every build — reused, not reimplemented.

## Documentation

| | |
| :--- | :--- |
| Command and MCP reference | [`docs/command-reference.md`](docs/command-reference.md) |
| Quick start (`--help` text) | [`crates/diskgraph-cli/QUICKSTART.md`](crates/diskgraph-cli/QUICKSTART.md) |
| Release channels and runbook | [`RELEASING.md`](RELEASING.md) |
| Acceptance records behind every number | [`docs/acceptance/`](docs/acceptance/) |
| Architecture / technical design | [`docs/DiskGraph-Architecture.md`](docs/DiskGraph-Architecture.md) · [设计](docs/DiskGraph-Architecture.zh_CN.md) |
| Requirements (the formal source) | [OpenSpec change](openspec/changes/implement-diskgraph-platform/proposal.md) — 14 specs, 77 requirements, 121 tasks |

## Contributing

Issues and pull requests are welcome. Run the gates before you push:

```bash
scripts/gates.sh
```

## License

[MIT](LICENSE), © 2026 Loong Wan. Includes a vendored copy of disktree-core (MIT, © Tobi Lütke) at revision `158f9cc` — see [`crates/diskgraph-disktree-core/VENDORED.md`](crates/diskgraph-disktree-core/VENDORED.md).
